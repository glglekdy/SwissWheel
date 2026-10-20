//! Notification text for a finished job.
//!
//! Phrased without Korean particles after the format name ("PNG 변환 완료"),
//! so we never have to guess 로/으로 for an English label.

use crate::catalog::Operation;

const OPEN_HINT: &str = "클릭하면 폴더를 엽니다";

/// (title, body) for a job where `succeeded` files worked and `failures`
/// holds one user-facing message per failed file.
pub fn job_summary(op: &Operation, succeeded: usize, failures: &[String]) -> (String, String) {
    let label = match op {
        Operation::Convert(f) => format!("{} 변환", f.label()),
        other => other.label().to_string(),
    };
    let Some(first_error) = failures.first() else {
        return (
            format!("{label} 완료"),
            format!("파일 {succeeded}개 · {OPEN_HINT}"),
        );
    };
    let title = format!("{label}: {}개 실패", failures.len());
    let body = if succeeded > 0 {
        format!("{first_error}\n성공 {succeeded}개 · {OPEN_HINT}")
    } else {
        first_error.clone()
    };
    (title, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Tool;
    use crate::format::Format;

    #[test]
    fn all_succeeded() {
        let (t, b) = job_summary(&Operation::Convert(Format::Png), 3, &[]);
        assert_eq!(t, "PNG 변환 완료");
        assert_eq!(b, "파일 3개 · 클릭하면 폴더를 엽니다");
        let (t, _) = job_summary(&Operation::Tool(Tool::HalveImage), 1, &[]);
        assert_eq!(t, "50% 축소 완료");
    }

    #[test]
    fn partial_and_total_failure() {
        let errs = vec![
            "사진.png: 이미지를 읽을 수 없습니다".to_string(),
            "b.png: x".to_string(),
        ];
        let (t, b) = job_summary(&Operation::Convert(Format::Jpg), 1, &errs);
        assert_eq!(t, "JPG 변환: 2개 실패");
        assert_eq!(
            b,
            "사진.png: 이미지를 읽을 수 없습니다\n성공 1개 · 클릭하면 폴더를 엽니다"
        );
        let (_, b) = job_summary(&Operation::Convert(Format::Jpg), 0, &errs[..1]);
        assert_eq!(b, "사진.png: 이미지를 읽을 수 없습니다");
    }
}
