use baba_sim::testkit::level;

#[test]
fn baba_moves() {
    let mut l = level(&["BABA IS YOU . .", ". . . . .", "baba> . . . ."], &[]);
    eprintln!("{:?}", l.rule_strings());
    l.play("R");
    assert_eq!(l.find_units("baba"), vec![(2, 3, 0)]);
}
