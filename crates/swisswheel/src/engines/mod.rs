//! Conversion engines. Each writes its result to a temp path chosen by the job runner.

pub mod image;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("read failed: {0}")]
    Read(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("write failed: {0}")]
    Write(String),
    #[error("engine failed: {0}")]
    Failed(String),
}

impl EngineError {
    /// One line for the notification, prefixed with the file name.
    pub fn user_message(&self, file_name: &str) -> String {
        let what = match self {
            EngineError::Read(_) => "이미지를 읽을 수 없습니다",
            EngineError::Unsupported(_) => "지원하지 않는 작업입니다",
            EngineError::Write(_) => "결과 파일을 쓸 수 없습니다",
            EngineError::Failed(_) => "변환에 실패했습니다",
        };
        format!("{file_name}: {what}")
    }
}
