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
