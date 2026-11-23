use super::*;

#[test]
fn slow_frames_shrink_and_quick_ones_grow_within_bounds() {
    assert_eq!(next_width(1280, Duration::from_secs(1)), 640);
    assert_eq!(next_width(1280, Duration::from_millis(300)), 1280);
    assert_eq!(next_width(1280, Duration::from_millis(50)), 1600);
    assert_eq!(next_width(200, Duration::from_secs(1)), MIN_FRAME_WIDTH);
    assert_eq!(next_width(MAX_FRAME_WIDTH, Duration::ZERO), MAX_FRAME_WIDTH);
}

#[test]
fn clicks_map_into_the_screen_and_miss_the_letterbox() {
    // A 2000x1000 screen drawn in a 1000x1000 area: 1000x500, 250 from the top.
    let area = (0., 0., 1000., 1000.);
    let screen = (2000, 1000);
    assert_eq!(screen_point((500., 500.), area, screen), Some((1000, 500)));
    assert_eq!(screen_point((0., 250.), area, screen), Some((0, 0)));
    assert_eq!(screen_point((500., 100.), area, screen), None);
    assert_eq!(screen_point((500., 900.), area, screen), None);
    assert_eq!(screen_point((10., 10.), (0., 0., 0., 0.), screen), None);
}

#[test]
fn keys_have_the_names_machine_key_takes() {
    assert_eq!(key_name("enter"), Some("return"));
    assert_eq!(key_name("backspace"), Some("delete"));
    assert_eq!(key_name("f13"), None);
}
