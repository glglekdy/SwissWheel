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

/// "999 B", "1.5 KB", "12 MB": one decimal below 10, whole numbers above.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    // Rounding can reach the next unit (1023.97 KB → "1024 KB"); carry it over.
    let shown = if value < 9.95 {
        (value * 10.0).round() / 10.0
    } else {
        value.round()
    };
    if shown >= 1024.0 && unit < UNITS.len() - 1 {
        return format!("1.0 {}", UNITS[unit + 1]);
    }
    if shown < 10.0 {
        format!("{shown:.1} {}", UNITS[unit])
    } else {
        format!("{shown:.0} {}", UNITS[unit])
    }
}

/// What the wheel's center shows about the dragged items. Folder contents
/// aren't summed (that would mean walking the tree on the UI thread).
pub fn selection_label(files: usize, folders: usize, file_bytes: u64) -> String {
    match (files, folders) {
        (0, 0) => String::new(),
        (1, 0) => format_size(file_bytes),
        (n, 0) => format!("{n}개 · {}", format_size(file_bytes)),
        (0, n) => format!("폴더 {n}개"),
        (f, d) => format!("{}개", f + d),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Tool;
    use crate::format::Format;

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(999), "999 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(2_516_582), "2.4 MB");
        assert_eq!(format_size(12 * 1024 * 1024), "12 MB");
        assert_eq!(format_size(1024 * 1024 - 1), "1.0 MB"); // 1023.999 KB must not print "1024 KB"
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn selection_labels() {
        assert_eq!(selection_label(1, 0, 2_516_582), "2.4 MB");
        assert_eq!(selection_label(3, 0, 7_444_889), "3개 · 7.1 MB");
        assert_eq!(selection_label(0, 2, 0), "폴더 2개");
        assert_eq!(selection_label(2, 1, 1024), "3개");
        assert_eq!(selection_label(0, 0, 0), "");
    }

    #[test]
    fn all_succeeded() {
        let (t, b) = job_summary(&Operation::Convert(Format::Png), 3, &[]);
        assert_eq!(t, "PNG 변환 완료");
        assert_eq!(b, "파일 3개 · 클릭하면 폴더를 엽니다");
        let (t, _) = job_summary(&Operation::Tool(Tool::Shrink), 1, &[]);
        assert_eq!(t, "축소 완료");
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
