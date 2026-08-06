//! `x.ai/feedback`, `x.ai/feedback/dismiss`, `x.ai/btw`, `x.ai/optimize_prompt`,
//! and `x.ai/review/*` extension handlers.
//!
//! - `feedback`/`feedback/dismiss`: persist user ratings/text locally and
//!   forward to cli-chat-proxy.
//! - `btw`: dispatch a side question to the active session via
//!   `SessionCommand::SideQuestion` and return the answer.
//! - `optimize_prompt`: rewrite a composer draft (Optimize input sparkle) via
//!   `SessionCommand::OptimizePrompt` (or a session-less aux call when no session).
//! - `review/comment` and `review/comment/delete`: record inline code review
//!   events to cloud storage.

use std::sync::Arc;

use agent_client_protocol as acp;
use tokio::sync::oneshot;

use super::{ExtResult, parse_params};
use crate::agent::MvpAgent;
use crate::session::persistence::{LocalFeedbackEntry, UserFeedbackEntry};
use crate::session::{
    ClientFeedbackInput, CommentDeleteRequest, CommentDeleteResponse, CommentRequest,
    CommentResponse, FeedbackRequestDismiss, FeedbackResponse, SessionCommand, SideQuestionError,
};
use crate::upload::gcs::WithAuth as _;
use file_utils::gcs::upload_bytes;
use intellihelper_telemetry::id::agent_id;

#[tracing::instrument(skip_all, fields(method = %args.method))]
pub async fn handle(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    match args.method.as_ref() {
        "x.ai/btw" => {
            tracing::info!("handling /btw side question");
            handle_btw(agent, args).await
        }
        "x.ai/optimize_prompt" => {
            tracing::info!("handling optimize_prompt");
            handle_optimize_prompt(agent, args).await
        }
        "x.ai/feedback" | "x.ai/feedback/dismiss" => {
            tracing::info!("handling user feedback");
            handle_feedback(agent, args).await
        }
        m if m.starts_with("x.ai/review") => {
            tracing::info!("handling review comment");
            handle_review(agent, args).await
        }
        _ => Err(acp::Error::method_not_found()),
    }
}

/// Handle `x.ai/optimize_prompt` — rewrite a composer draft without sending a turn.
async fn handle_optimize_prompt(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OptimizeRequest {
        #[serde(default)]
        session_id: Option<String>,
        draft: String,
        #[serde(default)]
        cwd: Option<String>,
    }

    let req: OptimizeRequest = parse_params(args)?;
    if req.draft.trim().is_empty() {
        return Err(acp::Error::invalid_params().data("draft is empty"));
    }

    // Prefer the live session path (cwd + model from the actor).
    if let Some(session_id) = req.session_id.as_deref().filter(|s| !s.is_empty()) {
        let sid: acp::SessionId = session_id.to_owned().into();
        let Some(session) = agent.resident_handle(&sid) else {
            return Err(
                acp::Error::invalid_params().data(format!("session not found: {session_id}"))
            );
        };
        let (tx, rx) = oneshot::channel();
        let _ = session.cmd_tx.send(SessionCommand::OptimizePrompt {
            draft: req.draft,
            respond_to: tx,
        });
        let result = rx
            .await
            .map_err(|_| acp::Error::internal_error().data("session failed to respond"))?;
        return map_optimize_result(result);
    }

    // Welcome / no session: aux sampling + optional cwd snapshot.
    let cwd = req
        .cwd
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
    match optimize_prompt_standalone(agent, &req.draft, &cwd).await {
        Ok(optimized) => super::to_ext_response(Ok(serde_json::json!({
            "optimized": optimized,
        }))),
        Err(SideQuestionError::Sampling(e)) => {
            Err(crate::sampling::error::map_sampling_err_to_acp(e))
        }
        Err(e) => Err(acp::Error::new(
            acp::ErrorCode::InternalError.into(),
            e.to_string(),
        )),
    }
}

fn map_optimize_result(result: Result<String, SideQuestionError>) -> ExtResult {
    match result {
        Ok(optimized) => super::to_ext_response(Ok(serde_json::json!({
            "optimized": optimized,
        }))),
        Err(SideQuestionError::Sampling(e)) => {
            Err(crate::sampling::error::map_sampling_err_to_acp(e))
        }
        Err(e) => Err(acp::Error::new(
            acp::ErrorCode::InternalError.into(),
            e.to_string(),
        )),
    }
}

/// Session-less optimize: build an aux sampling client and rewrite the draft.
async fn optimize_prompt_standalone(
    agent: &MvpAgent,
    draft: &str,
    cwd: &std::path::Path,
) -> Result<String, SideQuestionError> {
    use crate::session::helpers::optimize_prompt::{
        OPTIMIZE_SYSTEM_PROMPT, build_user_message, gather_workspace_context, sanitize_optimized,
    };
    use crate::sampling::{ConversationItem, ConversationRequest};

    let (client, model) = agent
        .build_optimize_client()
        .map_err(|e| SideQuestionError::PrepareClient(e.to_string()))?;

    let cwd_owned = cwd.to_path_buf();
    let draft_for_ctx = draft.trim().to_string();
    let workspace_context: String = tokio::task::spawn_blocking(move || {
        gather_workspace_context(&cwd_owned, &draft_for_ctx)
    })
    .await
    .unwrap_or_else(|_| String::new());

    let items = vec![
        ConversationItem::system(OPTIMIZE_SYSTEM_PROMPT.to_owned()),
        ConversationItem::user(build_user_message(draft.trim(), &workspace_context)),
    ];
    let request = ConversationRequest {
        items,
        tools: vec![],
        model: Some(model),
        temperature: Some(0.3),
        max_output_tokens: Some(512),
        x_intellihelper_conv_id: Some(format!("optimize-{}", uuid::Uuid::new_v4())),
        x_intellihelper_req_id: Some(format!("xai-optimize-{}", uuid::Uuid::new_v4())),
        x_intellihelper_agent_id: Some(agent_id()),
        ..Default::default()
    };
    let response = client
        .conversation_collect(request)
        .await
        .map_err(SideQuestionError::from)?;
    let raw = response.assistant_text();
    sanitize_optimized(&raw).map_err(|_| SideQuestionError::EmptyResponse)
}

/// Handle `x.ai/btw` -- a side question that doesn't interrupt the current turn.
async fn handle_btw(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct BtwRequest {
        session_id: String,
        question: String,
    }

    let req: BtwRequest = parse_params(args)?;
    let sid: acp::SessionId = req.session_id.clone().into();
    let session_handle = agent.resident_handle(&sid);
    let Some(session) = session_handle else {
        return Err(
            acp::Error::invalid_params().data(format!("session not found: {}", req.session_id))
        );
    };
    let (tx, rx) = oneshot::channel();
    let _ = session.cmd_tx.send(SessionCommand::SideQuestion {
        question: req.question,
        respond_to: tx,
    });
    let result = rx
        .await
        .map_err(|_| acp::Error::internal_error().data("session failed to respond"))?;
    match result {
        Ok(answer) => super::to_ext_response(Ok(serde_json::json!({
            "answer": answer,
        }))),
        // Model errors take the canonical mapping: overload gets its short
        // display copy there, rate limits keep the typed code + upgrade
        // copy, auth failures surface as auth_required.
        Err(SideQuestionError::Sampling(e)) => {
            Err(crate::sampling::error::map_sampling_err_to_acp(e))
        }
        // Non-model failures are already readable sentences. Set `message`
        // and leave `data` unset — `Display` appends JSON-encoded `data`,
        // and `internal_error().data(e)` rendered as `Internal error: "…"`,
        // which made capacity failures look like client bugs in the TUI.
        Err(e) => Err(acp::Error::new(
            acp::ErrorCode::InternalError.into(),
            e.to_string(),
        )),
    }
}

async fn handle_feedback(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    if !agent.cfg.borrow().is_feedback_enabled() {
        return Err(acp::Error::internal_error().data(
            "Feedback is disabled. To enable, set INTELLIHELPER_FEEDBACK_ENABLED=true or \
             [features] feedback = true in config.toml.",
        ));
    }

    match args.method.as_ref() {
        "x.ai/feedback" => {
            // Parse the input -- try the full ClientFeedbackInput first,
            // then fall back to the simple FeedbackRequest (from /feedback slash command)
            // which only has {session_id, feedback_text} and no client_type.
            let feedback_input: ClientFeedbackInput =
                match serde_json::from_str::<ClientFeedbackInput>(args.params.get()) {
                    Ok(input) => input,
                    Err(_) => {
                        // Fallback: parse simple FeedbackRequest from /feedback command
                        let simple: crate::session::FeedbackRequest = parse_params(args)?;
                        ClientFeedbackInput {
                            session_id: simple.session_id,
                            client_type:
                                prod_mc_cli_chat_proxy_types::feedback_types::ClientType::Tui,
                            rating_type: None,
                            rating_value: None,
                            feedback_text: Some(simple.feedback_text),
                            feedback_categories: vec![],
                            context_type: None,
                            turn_number: None,
                            request_id: None,
                            client_version: None,
                            metadata: None,
                            terminal_info: None,
                        }
                    }
                };

            let session_id = acp::SessionId::new(feedback_input.session_id.clone());
            let session_handle = agent.resident_handle(&session_id);

            let (model_id, model_metadata) = if let Some(ref session) = session_handle {
                let (tx1, rx1) = tokio::sync::oneshot::channel();
                let _ = session
                    .cmd_tx
                    .send(SessionCommand::GetCurrentModel { responds_to: tx1 });
                let model_id = rx1.await.ok();

                let model_metadata = session.get_model_metadata().await;

                (model_id, model_metadata)
            } else {
                let sampling_config = agent.sampling_config.borrow().clone();
                (Some(sampling_config.model.clone()), Default::default())
            };

            let turn_number = feedback_input.turn_number.or_else(|| {
                agent
                    .session_turn_number(&session_id)
                    .map(|t| t.saturating_sub(1) as i64)
            });

            let mut submission = feedback_input.to_submission(
                model_id.clone(),
                model_metadata.resolved_model_id,
                model_metadata.model_fingerprint,
                turn_number,
            );
            let turn_number = submission.turn_number;

            // Enrich with session context for Slack notifications (best-effort).
            if let Some(ref session_handle) = session_handle {
                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = session_handle
                    .cmd_tx
                    .send(SessionCommand::GetFeedbackContext {
                        turn_number,
                        responds_to: tx,
                    });
                if let Ok(ctx) = rx.await {
                    submission.tool_outcomes = ctx.tool_outcomes;
                    submission.session_cwd = Some(ctx.session_cwd);
                    submission.compaction_count = Some(ctx.compaction_count);
                    submission.context_window_usage = Some(ctx.context_window_usage);
                    submission.context_tokens_used = Some(ctx.context_tokens_used);
                    submission.context_window_tokens = Some(ctx.context_window_tokens);
                }
            }

            // Track rating in session signals
            if let (Some(session_handle), Some(rating_value)) =
                (&session_handle, feedback_input.rating_value)
            {
                use prod_mc_cli_chat_proxy_types::feedback_types::RatingType;
                let (is_positive, is_negative) = match feedback_input.rating_type {
                    // Thumbs: -1 = down, 0 = neutral, 1 = up
                    Some(RatingType::Thumbs) | None => (rating_value > 0, rating_value < 0),
                    // Stars (1-5): >= 4 positive, <= 2 negative, 3 neutral
                    Some(RatingType::Stars) => (rating_value >= 4, rating_value <= 2),
                    // NPS (0-10): 9-10 promoter, 0-6 detractor, 7-8 passive
                    Some(RatingType::Nps) => (rating_value >= 9, rating_value <= 6),
                };
                if is_positive {
                    session_handle.signals_handle.record_positive_rating();
                } else if is_negative {
                    session_handle.signals_handle.record_negative_rating();
                }
            }

            // Log feedback type for debugging
            if feedback_input.is_solicited() {
                tracing::info!(
                    session_id = %feedback_input.session_id,
                    request_id = ?feedback_input.request_id(),
                    turn_number = ?turn_number,
                    "Solicited feedback received (response to feedback request)"
                );
            } else {
                tracing::info!(
                    session_id = %feedback_input.session_id,
                    turn_number = ?turn_number,
                    "Spontaneous user feedback received"
                );
            }

            let telemetry_enabled = {
                let cfg = agent.cfg.borrow();
                cfg.is_telemetry_enabled()
                    && !agent
                        .auth_manager
                        .current_or_expired()
                        .is_some_and(|a| a.is_zdr_team())
            };
            let client = agent.feedback_client();
            if client.is_none() {
                tracing::warn!(
                    "no feedback client available (missing proxy credentials); feedback saved locally only"
                );
            }
            // Read the live feedback.user config (the session-actor path uses its
            // spawn-time snapshot); both dedupe through the same process-wide
            // identity cache, so a stable config resolves identically either way.
            // Clone out so the RefCell borrow doesn't span an await.
            let user_cfg = agent.cfg.borrow().feedback.user.clone();
            let author_identity =
                crate::util::user_identity::cached_identity(user_cfg.as_ref()).await;
            let outcome = crate::session::feedback_manager::submit_feedback_workflow(
                &mut submission,
                client.as_ref(),
                session_handle.as_ref().map(|h| &h.persistence_tx),
                crate::session::feedback_manager::SubmitFeedbackOptions {
                    solicited: feedback_input.is_solicited(),
                    telemetry_enabled,
                    author_identity,
                },
            )
            .await;

            match &outcome {
                crate::session::feedback_manager::SubmitOutcome::Submitted => {
                    tracing::info!("feedback submitted to proxy successfully");
                }
                crate::session::feedback_manager::SubmitOutcome::LocalOnly => {
                    tracing::warn!("feedback saved locally only (no proxy client)");
                }
                crate::session::feedback_manager::SubmitOutcome::Failed(e) => {
                    tracing::error!(error = %e, "feedback submission to proxy failed");
                    return Err(acp::Error::internal_error()
                        .data(format!("Feedback submission failed: {e}")));
                }
            }

            let value = serde_json::to_value(FeedbackResponse { success: true })
                .map(|value| serde_json::value::to_raw_value(&value).map(Arc::from))
                .expect("to work")
                .expect("to work");
            Ok(acp::ExtResponse::new(value))
        }
        "x.ai/feedback/dismiss" => {
            let dismiss_input: FeedbackRequestDismiss = parse_params(args)?;

            tracing::info!(
                session_id = %dismiss_input.session_id,
                request_id = %dismiss_input.request_id,
                "Feedback request dismissed by user"
            );

            // Count dismissals too (else event_type is always "responded" and
            // response-rate is unknowable), gated like the responded path so a
            // ZDR team emits no survey data and the ratio stays comparable.
            let telemetry_enabled = {
                let cfg = agent.cfg.borrow();
                cfg.is_telemetry_enabled()
                    && !agent
                        .auth_manager
                        .current_or_expired()
                        .is_some_and(|a| a.is_zdr_team())
            };
            if telemetry_enabled {
                tracing::info_span!(
                    "feedback.survey",
                    survey_type = "session",
                    event_type = "dismissed",
                    appearance_id = %dismiss_input.request_id,
                    has_feedback_text = false,
                    is_solicited = true,
                )
                .in_scope(|| {});
            }

            // Persist dismiss locally; flushed before storage CopyFile by the persistence actor.
            {
                let session_id = acp::SessionId::new(dismiss_input.session_id.clone());
                if let Some(session_handle) = agent.resident_handle(&session_id) {
                    session_handle.persist_feedback(LocalFeedbackEntry::UserFeedback(
                        UserFeedbackEntry {
                            submitted_at: chrono::Utc::now(),
                            session_id: dismiss_input.session_id.clone(),
                            turn_number: None,
                            solicited: true,
                            request_id: Some(dismiss_input.request_id.clone()),
                            dismissed: true,
                            submission: None,
                        },
                    ));
                }
            }

            let request_id = dismiss_input.request_id.clone();
            let client = agent
                .feedback_client()
                .ok_or_else(|| acp::Error::internal_error().data("No credentials for feedback"))?;
            let feedback_base_url = agent.cfg.borrow().endpoints.resolve_feedback_base_url();
            match client.dismiss_request(&request_id).await {
                Ok(response) => {
                    tracing::info!(
                        request_id = %response.request_id,
                        status = %response.status,
                        feedback_url = %feedback_base_url,
                        "Feedback request dismissed"
                    );
                    let value = serde_json::to_value(&response)
                        .map(|value| serde_json::value::to_raw_value(&value).map(Arc::from))
                        .expect("to work")
                        .expect("to work");
                    Ok(acp::ExtResponse::new(value))
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        request_id = %request_id,
                        feedback_url = %feedback_base_url,
                        "Failed to dismiss feedback request"
                    );
                    Err(acp::Error::internal_error()
                        .data(format!("Failed to dismiss feedback request: {e}")))
                }
            }
        }
        _ => Err(acp::Error::method_not_found()),
    }
}

/// Record inline code review events.
///
/// Methods:
/// - `x.ai/review/comment`: record a new inline code comment to cloud storage
/// - `x.ai/review/comment/delete`: record a tombstone event for a deleted comment
async fn handle_review(agent: &MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    match args.method.as_ref() {
        "x.ai/review/comment" => {
            let request: CommentRequest = parse_params(args)?;

            let comment_id = uuid::Uuid::now_v7().to_string();

            tracing::info!(
                comment_id = %comment_id,
                session_id = %request.session_id,
                prompt_index = request.prompt_index,
                path = %request.citation.path,
                lines = %format!("{}-{}", request.citation.start_line, request.citation.end_line),
                "Comment received"
            );

            let record = serde_json::json!({
                "event": "create",
                "commentId": comment_id,
                "sessionId": request.session_id,
                "promptIndex": request.prompt_index,
                "comment": null,
                "citation": request.citation,
                "agentId": agent_id().to_string(),
                "clientType": format!("{:?}", agent.client_type()),
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });

            if let Some(gcs_config) = agent
                .build_gcs_config(format!("{}/comments", request.session_id))
                .await
            {
                let json_bytes = serde_json::to_vec_pretty(&record)
                    .map_err(|e| acp::Error::internal_error().data(e.to_string()))?;
                let gcs_path = format!(
                    "{}/{}.json",
                    gcs_config.gcs_prefix.as_deref().unwrap_or("comments"),
                    comment_id
                );

                let auth_manager = Some(agent.auth_manager.clone());
                tokio::spawn(async move {
                    match upload_bytes(
                        &gcs_config.with_auth(auth_manager),
                        &gcs_path,
                        &json_bytes,
                        "application/json",
                    )
                    .await
                    {
                        Ok(gcs_url) => {
                            tracing::info!(gcs_url = %gcs_url, "Comment uploaded to GCS");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, gcs_path, "Failed to upload comment to GCS");
                        }
                    }
                });
            }

            let value = serde_json::to_value(CommentResponse {
                comment_id,
                recorded: true,
            })
            .map(|value| serde_json::value::to_raw_value(&value).map(Arc::from))
            .expect("to work")
            .expect("to work");
            Ok(acp::ExtResponse::new(value))
        }
        "x.ai/review/comment/delete" => {
            let request: CommentDeleteRequest = parse_params(args)?;

            tracing::info!(
                comment_id = %request.comment_id,
                session_id = %request.session_id,
                "Comment delete received"
            );

            let record = serde_json::json!({
                "event": "delete",
                "commentId": request.comment_id,
                "sessionId": request.session_id,
                "agentId": agent_id().to_string(),
                "clientType": format!("{:?}", agent.client_type()),
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });

            if let Some(gcs_config) = agent
                .build_gcs_config(format!("{}/comments", request.session_id))
                .await
            {
                let json_bytes = serde_json::to_vec_pretty(&record)
                    .map_err(|e| acp::Error::internal_error().data(e.to_string()))?;
                let event_id = uuid::Uuid::now_v7().to_string();
                let gcs_path = format!(
                    "{}/{}.json",
                    gcs_config.gcs_prefix.as_deref().unwrap_or("comments"),
                    event_id
                );

                let auth_manager = Some(agent.auth_manager.clone());
                tokio::spawn(async move {
                    match upload_bytes(
                        &gcs_config.with_auth(auth_manager),
                        &gcs_path,
                        &json_bytes,
                        "application/json",
                    )
                    .await
                    {
                        Ok(gcs_url) => {
                            tracing::info!(gcs_url = %gcs_url, "Comment delete event uploaded to GCS");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, gcs_path, "Failed to upload comment delete event to GCS");
                        }
                    }
                });
            }

            let value = serde_json::to_value(CommentDeleteResponse {
                comment_id: request.comment_id,
                deleted: true,
            })
            .map(|value| serde_json::value::to_raw_value(&value).map(Arc::from))
            .expect("to work")
            .expect("to work");
            Ok(acp::ExtResponse::new(value))
        }
        _ => Err(acp::Error::method_not_found()),
    }
}
