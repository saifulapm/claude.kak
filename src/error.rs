use thiserror::Error;

#[derive(Error, Debug)]
pub enum KakClaude {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Kakoune message error: {0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, KakClaude>;
