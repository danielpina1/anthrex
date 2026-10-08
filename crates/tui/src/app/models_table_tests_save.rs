//! Milestone 9.8 task 10, fix round 1: the `this repo` scope's save when no reply comes
//! (expiry, a refused send, a lost link), its reply reconciled, and a new global cache
//! reaching a screen whose only edits are the repository's.

use super::tests::{repo_requests, row, screen, select_role, spec, tap};
use crate::app::App;
use crate::app::settings_screen::SaveOutcome;
use crossterm::event::KeyCode;
use proto::models::{ModelTable, Role};
use proto::{ClientMsg, DaemonMsg, RunReply, RunRequest, SettingsReply, SettingsRequest};

/// The spec's screen in `this repo` with an empty repository table read, then
/// `research`'s effort raised (an unsaved repository edit).
fn edited_repo() -> App {
    let mut app = spec();
    let project = app.goal_project().unwrap();
    let ask = repo_requests(&tap(&mut app, KeyCode::Right))[0].0;
    app.on_daemon(repo_reply(
        SettingsReply::RepoModels {
            project,
            table: ModelTable::default(),
            path: "/data/repos/tmp-1234/models.toml".into(),
        },
        ask,
    ));
    select_role(&mut app, Role::Research);
    tap(&mut app, KeyCode::Char('e'));
    app
}

fn repo_reply(reply: SettingsReply, id: u64) -> DaemonMsg {
    DaemonMsg::Run(RunReply::Settings {
        reply: Box::new(reply),
        request_id: Some(id),
    })
}

/// `w`'s `PutRepoModels`: its id and table.
fn save(app: &mut App) -> (u64, ModelTable) {
    match &repo_requests(&tap(app, KeyCode::Char('w')))[..] {
        [(id, SettingsRequest::PutRepoModels { table, .. })] => (*id, table.clone()),
        other => panic!("one PutRepoModels: {other:?}"),
    }
}

fn put_id(app: &App) -> Option<u64> {
    screen(app).models.repo.as_ref().and_then(|r| r.put_id)
}

#[test]
fn a_repo_save_with_no_reply_says_so_and_w_sends_again() {
    let mut app = edited_repo();
    let (id, _) = save(&mut app);
    app.set_reply_sent_at(
        id,
        std::time::Instant::now() - std::time::Duration::from_secs(60),
    );
    app.on_tick();
    assert_eq!(put_id(&app), None);
    assert_eq!(
        screen(&app).outcome,
        Some(SaveOutcome::Refused(vec!["no reply from daemon".into()]))
    );
    assert_eq!(app.toast_text(), None, "the screen says it");
    let (again, _) = save(&mut app);
    assert_ne!(again, id);
}

#[test]
fn a_repo_save_not_sent_or_cut_off_says_so_and_w_sends_again() {
    let mut app = edited_repo();
    let (id, table) = save(&mut app);
    let project = app.goal_project().unwrap();
    let unsent = |id: u64, table: ModelTable| ClientMsg::RunTagged {
        id,
        request: RunRequest::Settings(SettingsRequest::PutRepoModels {
            project: project.clone(),
            table,
        }),
    };
    assert!(app.on_send_failed(&unsent(id, table)).is_empty());
    assert_eq!(put_id(&app), None);
    assert_eq!(screen(&app).outcome, Some(SaveOutcome::NotSent));
    assert_eq!(app.toast_text(), None);

    save(&mut app);
    app.on_link_lost("gone");
    app.on_tick();
    assert_eq!(put_id(&app), None);
    assert_eq!(screen(&app).outcome, Some(SaveOutcome::LinkLost));
    let windows = app.windows.clone();
    app.on_reconnected(windows);
    save(&mut app);
}

#[test]
fn a_repo_save_reply_is_reconciled() {
    let mut app = edited_repo();
    // A refusal of another (stale) id leaves the save in flight.
    let (id, _) = save(&mut app);
    screen_put(&mut app, Some(id + 100));
    app.on_daemon(repo_reply(
        SettingsReply::Refused {
            problems: vec!["late".into()],
        },
        id,
    ));
    assert_eq!(put_id(&app), Some(id + 100));
    assert_eq!(screen(&app).outcome, None);

    // The daemon's stored table (normalised) is taken when nothing was edited since.
    let mut app = edited_repo();
    let (id, _) = save(&mut app);
    let project = app.goal_project().unwrap();
    let mut stored = ModelTable::default();
    stored.rows.insert(
        Role::Research,
        row("claude:claude-sonnet-5", Some("max"), None),
    );
    app.on_daemon(repo_reply(
        SettingsReply::RepoSaved {
            project,
            table: stored.clone(),
        },
        id,
    ));
    let repo = screen(&app).models.repo.clone().unwrap();
    assert_eq!((repo.table, repo.base), (stored.clone(), stored));
    assert!(!screen(&app).dirty());
}

fn screen_put(app: &mut App, id: Option<u64>) {
    if let Some(crate::app::screens::Screen::Settings(s)) = &mut app.screen
        && let Some(r) = &mut s.models.repo
    {
        r.put_id = id;
    }
}

/// Minor 2: the cache is the global document; a pending repository edit does not stop
/// an unchanged global table from following it.
#[test]
fn a_new_global_cache_reaches_a_screen_with_only_repo_edits() {
    let mut app = edited_repo();
    let mut other = crate::ui::settings::tests::sample();
    other.roles = super::tests::spec_roles();
    other.roles.rows.remove(&Role::Reviewer);
    let id = match app.settings_fetch() {
        crate::app::Effect::Send(ClientMsg::RunTagged { id, .. }) => id,
        e => panic!("{e:?}"),
    };
    app.on_daemon(repo_reply(
        SettingsReply::Current {
            doc: other.clone(),
            origin: Default::default(),
            path: "/cfg/config.toml".into(),
        },
        id,
    ));
    assert_eq!(screen(&app).models.global, other.roles);
    assert!(
        screen(&app).models.repo_dirty(),
        "the repository edit stays"
    );
}
