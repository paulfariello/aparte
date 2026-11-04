mod common;

use std::thread;
use std::time::Duration;

use common::{describe, row_text, wait_for_ready, wait_for_screen, Harness, COMMAND_ROW};

fn cpu_ticks(pid: u32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("read proc stat");
    let after_comm = stat.rsplit_once(')').expect("comm end").1;
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
}

/// Tab inside a password prompt must not spin the event loop, touch the typed password, or block Esc.
#[test]
fn tab_in_password_prompt_is_inert() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    h.send_command("/inject_msg hello");
    wait_for_screen(&h, "test-contact", Duration::from_secs(5));
    h.send_bytes(b"\x1b");
    assert!(wait_for_screen(&h, "NORMAL", Duration::from_secs(2)));
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b":connect tab-test@localhost\r");
    assert!(wait_for_screen(&h, "password:", Duration::from_secs(5)));

    h.send_bytes(b"ab");
    thread::sleep(Duration::from_millis(200));
    let before_tab = row_text(h.snapshot().screen(), COMMAND_ROW);
    h.send_bytes(b"\t");
    thread::sleep(Duration::from_millis(500));
    let after_tab = row_text(h.snapshot().screen(), COMMAND_ROW);
    assert_eq!(
        before_tab, after_tab,
        "Tab must leave the password buffer unchanged"
    );

    let pid = h.child.process_id().expect("child pid");
    let before = cpu_ticks(pid);
    thread::sleep(Duration::from_secs(1));
    let burnt = cpu_ticks(pid) - before;
    assert!(
        burnt < 30,
        "Tab in a password prompt spins the event loop: {} ticks/s",
        burnt
    );

    h.send_bytes(b"\x1b");
    assert!(wait_for_screen(&h, "NORMAL", Duration::from_secs(2)));
    let parser = h.snapshot();
    assert!(
        !row_text(parser.screen(), COMMAND_ROW).contains("password:"),
        "Esc must cancel the prompt after Tab\n{}",
        describe(parser.screen())
    );
    h.shutdown();
}
