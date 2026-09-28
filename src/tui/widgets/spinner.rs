/// The frame of the refresh spinner to show on tick `index`.
pub(crate) fn spinner_frame(index: usize) -> &'static str {
    const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
    FRAMES[index % FRAMES.len()]
}
