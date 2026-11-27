/// How long ago `t` was, as "3d ago"; empty for a time still to come.
pub fn ago(t: std::time::SystemTime) -> String {
    let Ok(dur) = t.elapsed() else {
        return String::new();
    };
    let secs = dur.as_secs();
    if secs < 60 {
        tr!("just now").into()
    } else if secs < 60 * 60 {
        tr!("{minutes}m ago", minutes = secs / 60)
    } else if secs < 60 * 60 * 24 {
        tr!("{hours}h ago", hours = secs / 3600)
    } else if secs < 60 * 60 * 24 * 30 {
        tr!("{days}d ago", days = secs / 86_400)
    } else {
        tr!("{months}mo ago", months = secs / (86_400 * 30))
    }
}
