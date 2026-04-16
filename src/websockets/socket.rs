use async_trait::async_trait;
use axum::extract::ws::{Message, WebSocket};
use futures::stream::StreamExt;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Simple WebSocket abstraction - all we care about is send/receive
#[async_trait]
pub trait SocketWrapper: Send {
    /// Send a text message to the client
    async fn send_message(&mut self, message: String) -> Result<(), SocketError>;

    /// Receive the next message from the client (None if connection closed)
    async fn receive_message(&mut self) -> Result<Option<String>, SocketError>;

    /// Send a ping frame to the client (for keepalive)
    async fn send_ping(&mut self) -> Result<(), SocketError>;

    /// Close the connection
    async fn close(&mut self) -> Result<(), SocketError>;
}

/// Handler for incoming WebSocket messages
#[async_trait]
pub trait MessageHandler: Send + Sync {
    /// Handle an incoming message from the client
    async fn handle_message(&self, username: &str, room_id: &str, message: String);
}

#[derive(Debug)]
pub enum SocketError {
    #[allow(dead_code)] // Error variant for future use
    ConnectionClosed,
    #[allow(dead_code)] // Error message for debugging
    SendFailed(String),
    #[allow(dead_code)] // Error message for debugging
    ReceiveFailed(String),
}

/// Direct implementation on axum's WebSocket
#[async_trait]
impl SocketWrapper for WebSocket {
    async fn send_message(&mut self, message: String) -> Result<(), SocketError> {
        self.send(Message::Text(message))
            .await
            .map_err(|e| SocketError::SendFailed(e.to_string()))
    }

    async fn receive_message(&mut self) -> Result<Option<String>, SocketError> {
        loop {
            match self.next().await {
                Some(Ok(Message::Text(text))) => return Ok(Some(text)),
                Some(Ok(Message::Binary(_))) => {
                    // Ignore unsupported binary messages
                    continue;
                }
                Some(Ok(Message::Ping(payload))) => {
                    if let Err(e) = self.send(Message::Pong(payload)).await {
                        return Err(SocketError::SendFailed(e.to_string()));
                    }
                    continue;
                }
                Some(Ok(Message::Pong(_))) => {
                    // Heartbeat acknowledgement - nothing to do
                    continue;
                }
                Some(Ok(Message::Close(_))) => return Ok(None),
                Some(Err(e)) => return Err(SocketError::ReceiveFailed(e.to_string())),
                None => return Ok(None), // Connection closed
            }
        }
    }

    async fn send_ping(&mut self) -> Result<(), SocketError> {
        self.send(Message::Ping(vec![]))
            .await
            .map_err(|e| SocketError::SendFailed(e.to_string()))
    }

    async fn close(&mut self) -> Result<(), SocketError> {
        self.send(Message::Close(None))
            .await
            .map_err(|e| SocketError::SendFailed(e.to_string()))
    }
}

/// Connection represents a managed WebSocket connection
/// It is used to send and receive messages to and from the client
/// The outbound receiver is a channel that receives messages from the ConnectionManager's outbound sender
pub struct Connection {
    pub username: String,
    pub room_id: String,
    socket: Box<dyn SocketWrapper>,
    outbound_receiver: mpsc::Receiver<String>,
    message_handler: Arc<dyn MessageHandler>,
}

impl Connection {
    pub fn new(
        username: String,
        room_id: String,
        socket: Box<dyn SocketWrapper>,
        outbound_receiver: mpsc::Receiver<String>,
        message_handler: Arc<dyn MessageHandler>,
    ) -> Self {
        Self {
            username,
            room_id,
            socket,
            outbound_receiver,
            message_handler,
        }
    }

    /// Run the connection - handles both sending and receiving until disconnect
    /// Implements keepalive by sending ping frames every 15 seconds (modern real-time apps standard)
    pub async fn run(mut self) -> Result<(), SocketError> {
        // Send ping every 15 seconds to keep connection alive (modern real-time apps standard)
        // This prevents intermediate proxies/NAT routers from timing out idle connections
        // and catches dead connections faster (Discord, Slack, multiplayer games use 10-15s)
        let mut ping_interval = tokio::time::interval(tokio::time::Duration::from_secs(15));
        // Skip the first tick (which fires immediately)
        ping_interval.tick().await;

        loop {
            tokio::select! {
                // Handle outbound messages (from our app to client)
                msg = self.outbound_receiver.recv() => {
                    match msg {
                        Some(message) => {
                            self.socket.send_message(message).await?
                        }
                        None => break, // Channel closed, disconnect
                    }
                }

                // Handle inbound messages (from client to our app)
                msg = self.socket.receive_message() => {
                    match msg {
                        Ok(Some(message)) => {
                            // Call the provided callback to handle the message
                            self.message_handler
                                .handle_message(&self.username, &self.room_id, message)
                                .await;
                        }
                        Ok(None) => break, // Client disconnected
                        Err(e) => return Err(e),
                    }
                }

                // Send periodic ping to keep connection alive
                _ = ping_interval.tick() => {
                    // Send ping frame - client will automatically respond with pong
                    // If this fails, the connection is likely dead
                    if let Err(e) = self.socket.send_ping().await {
                        tracing::debug!(
                            room_id = %self.room_id,
                            username = %self.username,
                            error = ?e,
                            "Failed to send keepalive ping, connection dead"
                        );
                        return Err(e);
                    }
                }
            }
        }

        // Clean disconnect with proper close frame (industry best practice)
        let _ = self.socket.close().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct TestSocket {
        sent: Arc<Mutex<Vec<String>>>,
        inbound: mpsc::Receiver<String>,
    }

    #[async_trait]
    impl SocketWrapper for TestSocket {
        async fn send_message(&mut self, message: String) -> Result<(), SocketError> {
            self.sent.lock().unwrap().push(message);
            Ok(())
        }

        async fn receive_message(&mut self) -> Result<Option<String>, SocketError> {
            Ok(self.inbound.recv().await)
        }

        async fn send_ping(&mut self) -> Result<(), SocketError> {
            // For testing, just track that ping was sent
            self.sent.lock().unwrap().push("__PING__".to_string());
            Ok(())
        }

        async fn close(&mut self) -> Result<(), SocketError> {
            Ok(())
        }
    }

    struct TestHandler {
        calls: Arc<Mutex<Vec<(String, String, String)>>>,
    }

    #[async_trait]
    impl MessageHandler for TestHandler {
        async fn handle_message(&self, username: &str, room_id: &str, message: String) {
            self.calls
                .lock()
                .unwrap()
                .push((username.to_string(), room_id.to_string(), message));
        }
    }

    #[tokio::test]
    async fn test_connection_sends_outbound_and_handles_inbound() {
        let (out_tx, out_rx) = mpsc::channel::<String>(4);

        let (in_tx, in_rx) = mpsc::channel::<String>(4);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let socket = TestSocket {
            sent: sent.clone(),
            inbound: in_rx,
        };

        let handler_calls = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(TestHandler {
            calls: handler_calls.clone(),
        });

        let conn = Connection::new(
            "user1".to_string(),
            "roomA".to_string(),
            Box::new(socket),
            out_rx,
            handler,
        );

        let join = tokio::spawn(conn.run());

        // Give the connection task a moment to start
        tokio::task::yield_now().await;

        // Send outbound to client
        out_tx.send("hello-out".to_string()).await.unwrap();

        // Send inbound from client
        in_tx.send("hello-in".to_string()).await.unwrap();

        // Wait until outbound has been sent and inbound handled
        let _ = tokio::time::timeout(std::time::Duration::from_millis(50), async {
            loop {
                let sent_len = { sent.lock().unwrap().len() };
                let calls_len = { handler_calls.lock().unwrap().len() };
                if sent_len >= 1 && calls_len >= 1 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await;

        // Close both sides after processing
        drop(out_tx);
        drop(in_tx);

        join.await.unwrap().unwrap();

        // Assert outbound was sent to socket
        let sent_vec = sent.lock().unwrap().clone();
        assert!(sent_vec.contains(&"hello-out".to_string()));

        // Assert handler was called with inbound
        let calls = handler_calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "user1");
        assert_eq!(calls[0].1, "roomA");
        assert_eq!(calls[0].2, "hello-in");
    }
}
