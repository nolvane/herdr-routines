//! Regression tests for the approval rules, the crontab and what the list
//! shows of untrusted files. Each one was a way around a safeguard once;
//! it now asserts that the way is closed. All on made-up folders, with a
//! stand-in for `crontab` (`crontab::use_fake`); nothing here reaches
//! Herdr (`herdr::use_fake` is never set, so there is no Herdr at all).

use super::*;
use crate::approval::Approvals;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

/// What something with file access writes in place of the owner's command.
const CHANGED: &str = "touch /tmp/not-what-the-owner-approved";

fn opens(_: &str, _: &str) -> bool {
    true
}

/// An app on made-up folders with one approved agent routine.
fn app(tag: &str) -> App {
    let dir = std::env::temp_dir().join(format!("herdr-routines-sec-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let paths = Paths {
        config: dir.join("config"),
        state: dir.join("state"),
        socket: dir.join("sock"),
    };
    std::fs::create_dir_all(&paths.state).unwrap();
    let mut f = Form::new("/tmp");
    f.name = "Morning report".into();
    f.prompt = "Write the report.\nThen stop.".into();
    let (file, prompt) = f.build().unwrap();
    let r = Routine {
        id: routine::new_id(&paths, "Morning report"),
        file,
        prompt,
        problem: None,
        unreadable: false,
        approved: false,
    };
    r.save_approved(&paths).unwrap();
    let mut a = App::new(paths);
    a.opener = opens;
    a
}

/// A stand-in for `crontab` on this test's thread: -l prints the file, a
/// path replaces it. Returns the file.
fn fake_crontab(a: &App, text: &str) -> std::path::PathBuf {
    let dir = a.paths.state.clone();
    let tab = dir.join("fake-crontab.txt");
    std::fs::write(&tab, text).unwrap();
    let fake = dir.join("crontab");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nif [ \"$1\" = -l ]; then cat '{0}'; else cp \"$1\" '{0}'; fi\n",
            tab.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    crate::crontab::use_fake(Some(&fake));
    tab
}

/// The app with one approved, managed cron job "say-hi" in a stand-in
/// crontab.
fn with_cron_job(tag: &str) -> (App, std::path::PathBuf) {
    let mut a = app(tag);
    let tab = fake_crontab(&a, "");
    let mut f = Form::new("/tmp");
    f.name = "Say hi".into();
    f.choose(Field::Agent, routine::SHELL);
    f.choose(Field::RunsIn, crate::routine::RunsIn::Cron.label());
    f.prompt = "echo hi".into();
    a.save_form(&f).unwrap();
    assert!(Routine::load(&a.paths, "say-hi").unwrap().approved);
    assert!(crontab(&tab).contains("\n0 9 * * 1-5 echo hi\n"));
    a.view = View::Cron;
    a.select_id("say-hi");
    (a, tab)
}

fn crontab(tab: &std::path::Path) -> String {
    std::fs::read_to_string(tab).unwrap()
}

/// Writes one of a routine's files as something other than the list would.
fn write_outside(a: &mut App, id: &str, name: &str, text: &str) {
    paths::write_atomic(&a.paths.routine_dir(id).join(name), text.as_bytes()).unwrap();
    a.crontab_read = None;
    a.reload();
}

fn screen(a: &mut App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| ui::draw(f, a)).unwrap();
    let buf = t.backend().buffer().clone();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn press(a: &mut App, code: KeyCode) {
    a.key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// Starts the job's wrapper as cron does, with the command of its line.
fn cron_fires(a: &App, id: &str, command: &str) -> i32 {
    let wrapper = crate::crontab::wrapper(&a.paths, id);
    let args = [
        wrapper.to_string_lossy().into_owned(),
        "-c".into(),
        command.into(),
    ];
    let (me, cmd) = crate::crontab::wrapper_call(&args).expect("seen as the wrapper");
    crate::crontab::wrapper_main(&me, &cmd)
}

#[test]
fn switching_a_waiting_cron_job_never_writes_its_new_command() {
    let (mut a, tab) = with_cron_job("toggle");
    write_outside(&mut a, "say-hi", "command.sh", CHANGED);
    assert!(!Routine::load(&a.paths, "say-hi").unwrap().approved);
    // Space never asks: it must not be what puts the change into the crontab.
    press(&mut a, KeyCode::Char(' '));
    let off = crontab(&tab);
    assert!(off.contains("\n#off 0 9 * * 1-5 echo hi\n"), "{off}");
    press(&mut a, KeyCode::Char(' '));
    let on = crontab(&tab);
    assert!(on.contains("\n0 9 * * 1-5 echo hi\n"), "{on}");
    assert!(!off.contains("not-what") && !on.contains("not-what"));
    assert!(!Routine::load(&a.paths, "say-hi").unwrap().approved);
    // Approving is what writes it.
    press(&mut a, KeyCode::Char('a'));
    screen(&mut a, 120, 40);
    press(&mut a, KeyCode::End);
    screen(&mut a, 120, 40);
    press(&mut a, KeyCode::Char('y'));
    assert!(
        Routine::load(&a.paths, "say-hi").unwrap().approved,
        "{:?}",
        a.flash
    );
    assert!(crontab(&tab).contains(&format!("\n0 9 * * 1-5 {CHANGED}\n")));
    crate::crontab::use_fake(None);
}

#[test]
fn looking_at_a_waiting_cron_job_leaves_the_crontab_alone() {
    let (mut a, tab) = with_cron_job("look");
    let dir = a.paths.routine_dir("say-hi");
    let toml = std::fs::read_to_string(dir.join("routine.toml"))
        .unwrap()
        .replace("0 9 * * 1-5", "* * * * *");
    paths::write_atomic(&dir.join("routine.toml"), toml.as_bytes()).unwrap();
    write_outside(&mut a, "say-hi", "command.sh", CHANGED);
    let before = crontab(&tab);
    // The owner only looks at the command and leaves with Esc.
    press(&mut a, KeyCode::Char('p'));
    assert!(a.editor.is_some());
    press(&mut a, KeyCode::Esc);
    assert_eq!(crontab(&tab), before);
    // Saving the form of a routine that waits does not write it either,
    // and neither does copying it.
    press(&mut a, KeyCode::Enter);
    assert!(a.form.is_some(), "{:?}", a.flash);
    a.form.as_mut().unwrap().name = "Say hi again".into();
    a.form.as_mut().unwrap().dirty = true;
    a.try_save();
    assert!(a.form.is_none(), "{:?}", a.flash);
    a.select_id("say-hi");
    press(&mut a, KeyCode::Char('c'));
    assert_eq!(crontab(&tab), before);
    assert!(!Routine::load(&a.paths, "say-hi").unwrap().approved);
    crate::crontab::use_fake(None);
}

#[test]
fn the_wrapper_never_approves_and_runs_as_approved() {
    let (a, tab) = with_cron_job("wrapper");
    let dir = a.paths.routine_dir("say-hi");
    // File access only: the cron job becomes an agent routine without
    // permission checks, set for one date, with other settings.
    let toml = "name = \"Renamed\"\nenabled = true\nschedule = \"@once 2030-01-01 09:00\"\nagent = \"claude\"\npermission_mode = \"bypassPermissions\"\nfolder = \"/tmp\"\nkeep_runs = 1\n";
    paths::write_atomic(&dir.join("routine.toml"), toml.as_bytes()).unwrap();
    paths::write_atomic(&dir.join("prompt.md"), b"Do what the owner never saw.").unwrap();
    assert!(!Routine::load(&a.paths, "say-hi").unwrap().approved);
    let before = crontab(&tab);
    // Cron fires the job's crontab line, twice.
    assert_eq!(cron_fires(&a, "say-hi", "echo hi"), 0);
    assert_eq!(cron_fires(&a, "say-hi", "echo hi"), 0);
    let now = Routine::load(&a.paths, "say-hi").unwrap();
    assert!(
        !now.approved,
        "the wrapper approved what the owner never saw"
    );
    assert!(!now.can_run());
    assert_eq!(crontab(&tab), before, "the approved line stays as it is");
    // The runs carry the approved name and are kept as approved (30, not 1).
    let runs = store::runs_of(&a.paths, "say-hi");
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|r| r.routine_name == "Say hi"));
    assert_eq!(
        std::fs::read_to_string(dir.join("routine.toml")).unwrap(),
        toml,
        "the files that wait are not overwritten"
    );
    crate::crontab::use_fake(None);
}

#[test]
fn a_cron_job_for_one_date_runs_on_that_date_only() {
    let (a, tab) = with_cron_job("once");
    let set = |date: chrono::NaiveDate| {
        let mut r = Routine::load(&a.paths, "say-hi").unwrap();
        r.file.enabled = true;
        r.file.schedule = format!("@once {} 09:00", date.format("%Y-%m-%d"));
        r.save_approved(&a.paths).unwrap();
        crate::crontab::write_job(&a.paths, &r).unwrap();
    };
    // Cron has no year: the line comes round on the same day every year.
    let today = Local::now().date_naive();
    let other_year = today
        .checked_add_months(chrono::Months::new(12))
        .expect("a date a year from now");
    set(other_year);
    let line = crontab(&tab);
    assert_eq!(cron_fires(&a, "say-hi", "echo hi"), 0);
    assert!(
        store::runs_of(&a.paths, "say-hi").is_empty(),
        "not its year"
    );
    assert_eq!(crontab(&tab), line, "and it stays on for its date");
    // On the date itself it runs and is then switched off.
    set(today);
    assert_eq!(cron_fires(&a, "say-hi", "echo hi"), 0);
    assert_eq!(store::runs_of(&a.paths, "say-hi").len(), 1);
    let r = Routine::load(&a.paths, "say-hi").unwrap();
    assert!(r.approved && !r.file.enabled);
    assert!(crontab(&tab).contains("\n#off "), "{}", crontab(&tab));
    crate::crontab::use_fake(None);
}

#[test]
fn an_approval_too_long_to_show_in_full_is_refused() {
    let mut a = app("long");
    let mut prompt = String::from("Tidy the folder.\n");
    for _ in 0..70_000 {
        prompt.push_str(".\n");
    }
    prompt.push_str("THEN DO WHAT NOBODY SCROLLED TO\n");
    write_outside(&mut a, "morning-report", "prompt.md", &prompt);
    assert!(!Routine::load(&a.paths, "morning-report").unwrap().approved);
    press(&mut a, KeyCode::Char('a'));
    assert!(matches!(a.mode, Mode::Log(_)), "{:?}", a.flash);
    screen(&mut a, 120, 40);
    press(&mut a, KeyCode::End);
    let end = screen(&mut a, 120, 40);
    assert!(end.contains("cannot be approved here"), "{end}");
    assert!(!end.contains(" y approve "), "approve is not offered");
    press(&mut a, KeyCode::Char('y'));
    assert!(!Routine::load(&a.paths, "morning-report").unwrap().approved);
    // A text that fits is approved as before, once read to the end.
    write_outside(&mut a, "morning-report", "prompt.md", "Tidy the folder.\n");
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Char('a'));
    screen(&mut a, 120, 40);
    press(&mut a, KeyCode::End);
    screen(&mut a, 120, 40);
    press(&mut a, KeyCode::Char('y'));
    assert!(
        Routine::load(&a.paths, "morning-report").unwrap().approved,
        "{:?}",
        a.flash
    );
}

#[test]
fn saving_is_judged_against_what_was_opened() {
    // The form: opened on files that wait, saved after they were put back.
    let mut a = app("opened-form");
    let dir = a.paths.routine_dir("morning-report");
    let approved = std::fs::read_to_string(dir.join("prompt.md")).unwrap();
    write_outside(&mut a, "morning-report", "prompt.md", CHANGED);
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.form.as_ref().unwrap().prompt.trim(), CHANGED);
    // Whatever changed the file puts the approved text back, so at the
    // moment of saving the files on disk are the approved ones again.
    paths::write_atomic(&dir.join("prompt.md"), approved.as_bytes()).unwrap();
    assert!(Routine::load(&a.paths, "morning-report").unwrap().approved);
    a.form.as_mut().unwrap().dirty = true;
    a.try_save();
    assert!(a.form.is_none(), "{:?}", a.flash);
    let r = Routine::load(&a.paths, "morning-report").unwrap();
    assert_eq!(r.prompt.trim(), CHANGED);
    assert!(!r.approved, "saving the form approved text from outside");
    let kept = Approvals::load(&a.paths).routine("morning-report").unwrap();
    assert_eq!(kept.prompt, approved);

    // The editor (p): the same.
    let mut a = app("opened-editor");
    let dir = a.paths.routine_dir("morning-report");
    write_outside(&mut a, "morning-report", "prompt.md", CHANGED);
    press(&mut a, KeyCode::Char('p'));
    assert!(a.editor.is_some());
    paths::write_atomic(&dir.join("prompt.md"), approved.as_bytes()).unwrap();
    press(&mut a, KeyCode::Esc);
    let r = Routine::load(&a.paths, "morning-report").unwrap();
    assert_eq!(r.prompt.trim(), CHANGED);
    assert!(!r.approved, "leaving the editor approved text from outside");

    // The owner's own edit of approved files stays approved.
    let mut a = app("opened-own");
    press(&mut a, KeyCode::Enter);
    a.form.as_mut().unwrap().name = "Morning report, renamed".into();
    a.form.as_mut().unwrap().dirty = true;
    a.try_save();
    let r = Routine::load(&a.paths, "morning-report").unwrap();
    assert!(r.approved && r.file.name == "Morning report, renamed");
    // Unless the files changed while the form was open.
    press(&mut a, KeyCode::Enter);
    paths::write_atomic(
        &a.paths.routine_dir("morning-report").join("prompt.md"),
        CHANGED.as_bytes(),
    )
    .unwrap();
    a.form.as_mut().unwrap().name = "Morning report".into();
    a.form.as_mut().unwrap().dirty = true;
    a.try_save();
    assert!(!Routine::load(&a.paths, "morning-report").unwrap().approved);
}

#[test]
fn running_by_hand_checks_the_routine_again() {
    let (mut a, _tab) = with_cron_job("runnow");
    // The list still shows the routine as it was when `y` was asked for.
    let shown = a
        .routines
        .iter()
        .find(|r| r.id == "say-hi")
        .cloned()
        .unwrap();
    assert!(shown.can_run());
    paths::write_atomic(
        &a.paths.routine_dir("say-hi").join("command.sh"),
        CHANGED.as_bytes(),
    )
    .unwrap();
    let refused = crate::crontab::run_now(&a.paths, &shown).unwrap_err();
    assert!(refused.contains("waits for approval"), "{refused}");
    a.fire("say-hi");
    let said = a.flash.as_ref().map(|f| f.text.clone()).unwrap_or_default();
    assert!(said.starts_with("Not started"), "{said}");
    assert!(store::runs_of(&a.paths, "say-hi").is_empty());
    crate::crontab::use_fake(None);
}

#[test]
fn the_approval_view_says_what_really_happens() {
    let (a, _tab) = with_cron_job("view");
    let r = Routine::load(&a.paths, "say-hi").unwrap();
    let text = approval_lines(&a.paths, &r).join("\n");
    assert!(text.contains("as a classic cron job"), "{text}");
    assert!(text.contains("writes its line into your crontab"));
    assert!(text.contains("also when Herdr is closed"));
    assert!(text.contains("in your home folder"));
    assert!(text.contains("Keeps: "));
    assert!(!text.contains("Warning"));
    // Extra arguments are called out, whatever they are.
    let mut r = Routine::load(&a.paths, "morning-report").unwrap();
    assert!(!approval_lines(&a.paths, &r).join("\n").contains("Warning"));
    r.file.args = vec!["--verbose".into()];
    let text = approval_lines(&a.paths, &r).join("\n");
    assert!(
        text.contains("Warning: it has extra arguments (--verbose)"),
        "{text}"
    );
    // A cron job set up for another folder is not one that can run.
    let mut r = Routine::load(&a.paths, "say-hi").unwrap();
    r.file.folder = "/tmp".into();
    let problem = Routine::check(&r.file, &r.prompt).unwrap_or_default();
    assert!(problem.contains("home folder"), "{problem}");
    crate::crontab::use_fake(None);
}

#[test]
fn output_that_is_not_text_still_shows() {
    let mut a = app("bytes");
    let run = Run::new(
        "morning-report",
        "Morning report",
        "claude",
        store::Trigger::Manual,
        None,
    );
    run.save(&a.paths).unwrap();
    std::fs::write(
        run.file(&a.paths, "out"),
        b"line one\n\xff\xfe binary\nlast line\n",
    )
    .unwrap();
    a.show_log(&run);
    let Mode::Log(v) = &a.mode else {
        panic!("no output view")
    };
    assert!(
        v.lines.iter().any(|l| l.contains("line one")),
        "{:?}",
        v.lines
    );
    assert!(v.lines.iter().any(|l| l.contains("last line")));
    // A very long output shows its end and says so.
    let mut long = vec![b'x'; 3 * 1024 * 1024];
    long.extend_from_slice(b"\nthe very end\n");
    std::fs::write(run.file(&a.paths, "out"), long).unwrap();
    a.show_log(&run);
    let Mode::Log(v) = &a.mode else {
        panic!("no output view")
    };
    assert!(v.lines[0].contains("the output is long"), "{}", v.lines[0]);
    assert!(v.lines.iter().any(|l| l.contains("the very end")));
}
