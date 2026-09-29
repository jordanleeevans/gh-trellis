use ratatui::layout::Rect;

/// A rect centered within `area`, `percent_x`/`percent_y` of its size, but
/// never smaller than `min_width` x `min_height` and never larger than
/// `area`, so overlays stay readable on small terminals.
pub(crate) fn centered_rect_min(
    area: Rect,
    percent_x: u16,
    percent_y: u16,
    min_width: u16,
    min_height: u16,
) -> Rect {
    let scaled = |total: u16, percent: u16, min: u16| {
        let wanted = u32::from(total) * u32::from(percent.min(100)) / 100;
        u16::try_from(wanted)
            .unwrap_or(u16::MAX)
            .max(min)
            .min(total)
    };
    let width = scaled(area.width, percent_x, min_width);
    let height = scaled(area.height, percent_y, min_height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_minimum_size_when_percentages_are_tiny() {
        let rect = centered_rect_min(Rect::new(0, 0, 60, 20), 60, 8, 44, 8);
        assert_eq!((rect.width, rect.height), (44, 8));
        assert_eq!((rect.x, rect.y), (8, 6));
    }

    #[test]
    fn never_exceeds_the_area() {
        let area = Rect::new(2, 3, 30, 5);
        let rect = centered_rect_min(area, 90, 90, 44, 10);
        assert_eq!(rect, area);
    }

    #[test]
    fn uses_the_percentage_when_it_is_larger() {
        let rect = centered_rect_min(Rect::new(0, 0, 160, 45), 50, 50, 20, 5);
        assert_eq!((rect.width, rect.height), (80, 22));
    }
}
