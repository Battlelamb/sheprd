use super::*;

#[test]
fn typing_before_the_first_key_stops_every_key() {
    let stage = Stage::default();
    assert!(stage.typed());
    assert!(!stage.interrupt());
    assert!(!stage.interrupt());
    assert!(stage.stopped_by_typing());
}

#[test]
fn typing_after_the_first_key_lets_the_move_finish() {
    let stage = Stage::default();
    assert!(stage.interrupt());
    assert!(!stage.typed(), "the agent is already stopping");
    assert!(stage.interrupt(), "later rounds still press their keys");
    assert!(!stage.stopped_by_typing());
}

#[test]
fn typing_racing_the_first_key_has_exactly_one_winner() {
    for _ in 0..200 {
        let stage = std::sync::Arc::new(Stage::default());
        let typist = {
            let stage = stage.clone();
            std::thread::spawn(move || stage.typed())
        };
        let pressed = stage.interrupt();
        let typed = typist.join().unwrap();
        assert_ne!(pressed, typed);
    }
}
