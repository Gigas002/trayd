use std::io::Write;

use super::{HELD, LineWriter, MAX_HELD, pause, resume};

fn write_line(text: &str) {
    let mut w = LineWriter(Vec::new());
    w.write_all(text.as_bytes()).unwrap();
}

fn held_lines() -> Vec<String> {
    HELD.lock()
        .unwrap()
        .as_ref()
        .map(|q| {
            q.iter()
                .map(|l| String::from_utf8_lossy(l).into_owned())
                .collect()
        })
        .unwrap_or_default()
}

// One test, because the paused state is process-global.
#[test]
fn paused_output_is_held_back_bounded_and_released() {
    pause();
    write_line("first\n");
    assert_eq!(held_lines(), ["first\n"]);

    for n in 0..MAX_HELD + 5 {
        write_line(&format!("{n}\n"));
    }
    let held = held_lines();
    assert_eq!(held.len(), MAX_HELD);
    assert_eq!(held.last().unwrap(), &format!("{}\n", MAX_HELD + 4));

    resume();
    assert!(HELD.lock().unwrap().is_none());
}
