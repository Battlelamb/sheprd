use super::{Answer, Answered, Ask};

#[test]
fn closing_unanswered_declines_and_the_first_answer_stays() {
    let answered = Answered::default();
    let ask = Ask {
        host: "mini-2".to_owned(),
        key: "mini-2".to_owned(),
        machine: "default".to_owned(),
        answered: answered.clone(),
    };
    drop(ask);
    assert_eq!(answered.take(), Some(Answer::Dismissed));
    assert_eq!(answered.take(), None);
    let answered = Answered::default();
    answered.set(Answer::Once);
    answered.set(Answer::Dismissed);
    assert_eq!(answered.take(), Some(Answer::Once));
}

#[test]
fn a_cancelled_request_is_seen_by_its_question_only() {
    let answered = Answered::default();
    let other = Answered::default();
    answered.clone().cancel();
    assert!(answered.cancelled());
    assert!(!other.cancelled());
    assert!(answered.same(&answered.clone()));
    assert!(!answered.same(&other));
}
