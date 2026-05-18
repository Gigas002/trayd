use super::*;

#[test]
fn pick_pixmap_exact_match() {
    let frames = vec![
        (16, 16, vec![0u8; 16 * 16 * 4]),
        (32, 32, vec![0u8; 32 * 32 * 4]),
        (64, 64, vec![0u8; 64 * 64 * 4]),
    ];
    let pix = pick_pixmap(frames, 32).unwrap();
    assert_eq!(pix.width, 32);
    assert_eq!(pix.height, 32);
}

#[test]
fn pick_pixmap_selects_closest_larger() {
    let frames = vec![
        (16, 16, vec![0u8; 16 * 16 * 4]),
        (64, 64, vec![0u8; 64 * 64 * 4]),
    ];
    // 24 is between 16 and 64; the closest larger one is 64.
    let pix = pick_pixmap(frames, 24).unwrap();
    assert_eq!(pix.width, 64);
}

#[test]
fn pick_pixmap_falls_back_to_first_when_empty() {
    let pix = pick_pixmap(vec![], 32);
    assert!(pix.is_none());
}

#[test]
fn pick_pixmap_single_frame() {
    let frames = vec![(48, 48, vec![0u8; 48 * 48 * 4])];
    let pix = pick_pixmap(frames, 22).unwrap();
    assert_eq!(pix.width, 48);
}

#[test]
fn scroll_orientation_vertical() {
    assert_eq!(scroll_orientation(ScrollDirection::Up), "vertical");
    assert_eq!(scroll_orientation(ScrollDirection::Down), "vertical");
}

#[test]
fn scroll_orientation_horizontal() {
    assert_eq!(scroll_orientation(ScrollDirection::Left), "horizontal");
    assert_eq!(scroll_orientation(ScrollDirection::Right), "horizontal");
}

#[test]
fn scroll_delta_signs() {
    assert!(scroll_delta(ScrollDirection::Up, 3) > 0);
    assert!(scroll_delta(ScrollDirection::Down, 3) < 0);
    assert!(scroll_delta(ScrollDirection::Right, 2) > 0);
    assert!(scroll_delta(ScrollDirection::Left, 2) < 0);
}
