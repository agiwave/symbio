use tokio_util::sync::CancellationToken;

/// RAII guard for typing indicator - ensures typing is stopped when dropped
pub struct TypingGuard {
    cancel_token: Option<CancellationToken>,
    typing_task: Option<tokio::task::JoinHandle<()>>,
}

impl TypingGuard {
    /// Create a new typing guard that continuously sends typing indicator
    pub fn new(client: reqwest::Client, api_url: String, chat_id: String) -> Self {
        let cancel_token = CancellationToken::new();
        let cancel_token_clone = cancel_token.clone();

        let typing_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(4)) => {
                        let _ = client
                            .post(format!("{api_url}/sendChatAction"))
                            .json(&serde_json::json!({
                                "chat_id": chat_id,
                                "action": "typing"
                            }))
                            .send()
                            .await;
                    }
                    _ = cancel_token_clone.cancelled() => {
                        break;
                    }
                }
            }
        });

        Self {
            cancel_token: Some(cancel_token),
            typing_task: Some(typing_task),
        }
    }

    /// Stop the typing indicator immediately
    pub fn stop(&mut self) {
        if let Some(token) = self.cancel_token.take() {
            token.cancel();
        }
        self.typing_task.take();
    }
}

impl Drop for TypingGuard {
    fn drop(&mut self) {
        self.stop();
    }
}
