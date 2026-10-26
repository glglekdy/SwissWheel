//! Conversion engines. Each writes its result to a temp path chosen by the job runner.

pub mod ffmpeg;
pub mod image;
pub mod metadata;

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
    #[error("media read failed: {0}")]
    MediaRead(String),
    #[error("no audio stream")]
    NoAudio,
    #[error("no video stream")]
    NoVideo,
    #[error("ffmpeg not found")]
    MissingFfmpeg,
    #[error("cancelled")]
    Cancelled,
}

impl EngineError {
    /// One line for the notification, prefixed with the file name.
    pub fn user_message(&self, file_name: &str) -> String {
        let what = match self {
            EngineError::Read(_) => "이미지를 읽을 수 없습니다",
            EngineError::Unsupported(_) => "지원하지 않는 작업입니다",
            EngineError::Write(_) => "결과 파일을 쓸 수 없습니다",
            EngineError::Failed(_) => "변환에 실패했습니다",
            EngineError::MediaRead(_) => "영상·오디오 파일을 읽을 수 없습니다",
            EngineError::NoAudio => "오디오 트랙이 없습니다",
            EngineError::NoVideo => "영상 트랙이 없습니다",
            EngineError::MissingFfmpeg => "FFmpeg를 찾을 수 없습니다",
            EngineError::Cancelled => "취소되었습니다",
        };
        format!("{file_name}: {what}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_user_messages() {
        assert_eq!(
            EngineError::NoAudio.user_message("a.mp4"),
            "a.mp4: 오디오 트랙이 없습니다"
        );
        assert_eq!(
            EngineError::MissingFfmpeg.user_message("a.mp4"),
            "a.mp4: FFmpeg를 찾을 수 없습니다"
        );
        assert_eq!(
            EngineError::MediaRead("x".into()).user_message("a.mp4"),
            "a.mp4: 영상·오디오 파일을 읽을 수 없습니다"
        );
        assert_eq!(
            EngineError::Cancelled.user_message("a.mp4"),
            "a.mp4: 취소되었습니다"
        );
    }
}
