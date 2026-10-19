//! M9 screens and their glue into [`App`]: the "Play levels" menu
//! (`ui.md` §8.4) with custom levels, level packs, level codes (enter a
//! code, featured list, history, download → save / play; `levelcodes.md`
//! §3–§5, §8), single-level play, importing the original's save (menu
//! entry and first-start offer; `saves.md` §5) and the Lua notice of packs
//! with mods (`worldpacks.md` §4). The logic lives in [`crate::import`],
//! [`crate::levelcodes`] and [`crate::packs`]; this module only drives
//! menus, jobs and play sessions, so `app.rs` needs a handful of hooks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use baba_render::{RegionId, RgbaImage};

use super::{App, Mode, Then};
use crate::import;
use crate::levelcodes::net::{Job, JobKind, JobResult, Server};
use crate::levelcodes::store::{self, TEMP};
use crate::levelcodes::{self, LcError, ListPage};
use crate::menu::{Act, Item, Menu, MenuId, Style};
use crate::osk;
use crate::packs::{self, LevelEntry};
use crate::play::{Env, Play, SingleLevel};
use crate::ui::{Align, Ui};
use crate::world::WorldInfo;

/// Custom levels per page.
const PER_PAGE: usize = 8;
/// Upload is compiled in (feature `upload`, off by default).
pub const UPLOAD: bool = cfg!(feature = "upload");

/// M9 menus (inside [`MenuId::Ext`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtMenu {
    /// "Play levels" (`ui.md` §8.4).
    PlayLevels,
    /// "Select the level you want to play:".
    Custom {
        page: usize,
    },
    /// "Get new levels": code, featured, history.
    GetLevels,
    /// Code entry (`levelcodes.md` §1.2).
    EnterCode {
        text: String,
    },
    Featured {
        page: usize,
    },
    /// Waiting for a request (§2.3).
    Wait,
    /// "Level found!" (§3.5).
    Found,
    /// "Level saved!".
    Saved {
        stem: String,
    },
    Failed {
        list: bool,
        err: String,
    },
    /// Level history (§8).
    History,
    /// First start: "import your progress?".
    ImportOffer,
    /// Settings → import.
    Import,
    ImportDone,
    /// Starting a pack that ships Lua mods.
    LuaNotice {
        pack: String,
    },
    /// Pick a level to upload (feature `upload`).
    Upload {
        page: usize,
    },
    UploadResult {
        ok: bool,
        text: String,
    },
}

/// M9 actions (inside [`Act::Ext`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtAct {
    PlayCustom(usize),
    CustomPage(usize),
    SubmitCode,
    Download(String),
    FeaturedPage(usize),
    SaveDownloaded,
    PlayDownloaded,
    PlaySaved(String),
    RunImport,
    SkipImport,
    StartPack(String),
    LeaveSingle,
    Upload(usize),
    UploadPage(usize),
}

/// A level played on its own.
pub(super) struct Single {
    pub code: Option<String>,
    pub stem: String,
    pub dir: PathBuf,
    /// Upload after winning (feature `upload`, §6.1).
    pub upload: bool,
    pub uploaded_ok: bool,
}

/// M9 state kept by the app.
#[derive(Default)]
pub struct ExtState {
    job: Option<Job>,
    downloaded: Option<store::Downloaded>,
    featured: Option<ListPage>,
    images: HashMap<PathBuf, Option<RegionId>>,
    found: Option<import::Found>,
    scanned: bool,
    report: Option<import::Report>,
    pub(super) single: Option<Single>,
    /// Menus restored when a single level ends.
    back: Vec<(MenuId, (usize, usize))>,
    /// Upload in flight (level folder + stem).
    uploading: Option<(PathBuf, String)>,
}

fn ext(m: ExtMenu) -> MenuId {
    MenuId::Ext(m)
}

fn open(m: ExtMenu) -> Act {
    Act::Open(MenuId::Ext(m))
}

fn wrap(s: &str, width: usize, max_lines: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for w in s.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + w.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(w);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out.truncate(max_lines);
    out
}

impl App {
    fn user_dir(&self) -> Option<PathBuf> {
        self.cfg.data_dir.clone()
    }

    /// Pack lookups honour the data dir (`packs::open_world`).
    pub(super) fn open_world(&self, id: &str) -> Option<WorldInfo> {
        packs::open_world(&self.data, self.cfg.data_dir.as_deref(), id)
    }

    fn custom_levels(&self) -> Vec<LevelEntry> {
        packs::custom_levels(&self.data, self.cfg.data_dir.as_deref())
    }

    fn replace_top(&mut self, m: ExtMenu) {
        if matches!(self.menus.last(), Some((MenuId::Ext(_), _))) {
            self.menus.pop();
        }
        self.menus.push((ext(m), (0, 0)));
    }

    fn fail(&mut self, list: bool, err: &LcError) {
        self.log.push(format!("levelcode error {err}"));
        let m = ExtMenu::Failed {
            list,
            err: err.to_string(),
        };
        if matches!(self.menus.last(), Some((MenuId::Ext(ExtMenu::Wait), _))) {
            self.replace_top(m);
        } else {
            self.menus.push((ext(m), (0, 0)));
        }
    }

    // ------------------------------------------------------------ hooks

    /// Every tick: finished background requests.
    pub(super) fn ext_poll(&mut self) {
        let Some(r) = self.ext.job.as_ref().and_then(Job::poll) else {
            return;
        };
        self.ext.job = None;
        let waiting = matches!(self.menus.last(), Some((MenuId::Ext(ExtMenu::Wait), _)));
        match r {
            JobResult::Level(Ok(d)) => {
                self.log.push(format!("downloaded {}", d.code));
                store::push_history(
                    &mut self.profile.settings,
                    "get_d",
                    &d.code,
                    d.record.name(),
                );
                self.profile.save_settings();
                self.ext.downloaded = Some(d);
                if waiting {
                    self.replace_top(ExtMenu::Found);
                }
            }
            JobResult::List(Ok(p)) => {
                self.log.push(format!(
                    "featured page {} of {} ({} levels)",
                    p.page,
                    p.pages,
                    p.entries.len()
                ));
                let page = p.page;
                self.ext.featured = Some(p);
                if waiting {
                    self.replace_top(ExtMenu::Featured { page });
                }
            }
            JobResult::Level(Err(e)) => {
                if waiting {
                    self.fail(false, &e)
                }
            }
            JobResult::List(Err(e)) => {
                if waiting {
                    self.fail(true, &e)
                }
            }
            JobResult::Uploaded(r) => self.upload_done(r),
        }
    }

    /// First launch: offer the import when an original save with progress
    /// exists. True if the offer is shown.
    pub(super) fn ext_offer_import(&mut self) -> bool {
        let f = self.scan_import();
        if !f.is_some_and(|f| f.any_progress()) {
            return false;
        }
        self.mode = Mode::Title;
        self.audio.music("menu");
        self.menus = vec![(MenuId::Main, (0, 0)), (ext(ExtMenu::ImportOffer), (0, 0))];
        self.log.push("import offer".into());
        true
    }

    fn scan_import(&mut self) -> Option<&import::Found> {
        if !self.ext.scanned {
            self.ext.found = import::find();
            self.ext.scanned = true;
        }
        self.ext.found.as_ref()
    }

    /// Pack start: the Lua notice (once per pack). True if shown.
    pub(super) fn ext_pack_notice(&mut self, id: &str) -> bool {
        let Some(w) = self.open_world(id) else {
            return false;
        };
        if !packs::has_lua(&w.dir, &w.data) || !packs::lua_notice_due(&self.profile.settings, id) {
            return false;
        }
        self.menus.push((
            ext(ExtMenu::LuaNotice {
                pack: id.to_string(),
            }),
            (0, 0),
        ));
        true
    }

    /// A single level's play is over (its win, ending or conversion was
    /// recorded by `Play::apply`, the victory banner and the closing
    /// transition ran): back to the title and the menus it came from.
    pub(super) fn ext_single_done(&mut self, won: bool) {
        if let Some(s) = &mut self.ext.single {
            s.uploaded_ok = won;
        }
        self.run_then(Then::ToTitle);
    }

    /// After a return to the title: back to the menus a single level was
    /// started from (and the upload, if one was pending).
    pub(super) fn ext_after_title(&mut self) {
        let Some(s) = self.ext.single.take() else {
            return;
        };
        if !self.ext.back.is_empty() {
            self.menus = std::mem::take(&mut self.ext.back);
        }
        if s.upload && s.uploaded_ok {
            self.start_upload(&s.dir, &s.stem);
        }
    }

    // ------------------------------------------------------------ menus

    pub(super) fn build_ext(&self, id: &ExtMenu) -> Menu {
        let t = &self.texts;
        let mut m = Menu::new(Style::List);
        m.top = 30.0;
        m.min_width = 360.0;
        match id {
            ExtMenu::PlayLevels => {
                let slot = self.profile.last_slot();
                m.top = 60.0;
                m.header.push(format!(
                    "{} {}",
                    t.get("slot_current"),
                    self.slot_name(slot)
                ));
                let user = self.user_dir().is_some();
                m.item(
                    Item::new(
                        t.get("customlevels_play_singular"),
                        open(ExtMenu::Custom { page: 0 }),
                    )
                    .enabled(!self.custom_levels().is_empty()),
                );
                m.item(Item::new(
                    t.get("customlevels_play_pack"),
                    Act::Open(MenuId::Packs { page: 0 }),
                ));
                m.item(
                    Item::new(t.get("customlevels_play_get"), open(ExtMenu::GetLevels))
                        .enabled(user),
                );
                m.item(Item::new(
                    t.get("customlevels_play_getlist"),
                    open(ExtMenu::History),
                ));
                let up = Item::new(
                    t.get("editor_menu_upload"),
                    open(ExtMenu::Upload { page: 0 }),
                );
                m.item(if UPLOAD {
                    up.enabled(user)
                } else {
                    up.enabled(false).value("(not in this build)")
                });
                m.item(Item::new(
                    t.get("customlevels_play_changename"),
                    Act::Open(MenuId::Rename {
                        slot,
                        text: String::new(),
                    }),
                ));
                m.gap(12.0);
                m.item(Item::new(t.get("return"), Act::Close));
            }
            ExtMenu::Custom { page } | ExtMenu::Upload { page } => {
                let upload = matches!(id, ExtMenu::Upload { .. });
                let all = self.custom_levels();
                let list: Vec<(usize, &LevelEntry)> = all
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| !upload || e.user)
                    .collect();
                let pages = list.len().div_ceil(PER_PAGE).max(1);
                let page = (*page).min(pages - 1);
                m.top = 20.0;
                m.min_width = 520.0;
                m.header.push(if upload {
                    t.get("editor_upload_confirm")
                } else {
                    t.get("customlevels_single")
                });
                let slot = &self.profile.slots[self.profile.last_slot()];
                for (i, e) in list.iter().skip(page * PER_PAGE).take(PER_PAGE) {
                    let won = (e.user && slot.level_state(packs::LEVELS, &e.stem) >= 3)
                        || (!e.code.is_empty() && store::won(slot, &e.code).won);
                    let mut name = if e.name.is_empty() {
                        e.stem.clone()
                    } else {
                        levelcodes::cut_name(&e.name)
                    };
                    if won {
                        name.push_str(" \u{273D}");
                    }
                    let value = if !e.code.is_empty() {
                        e.code.clone()
                    } else if !e.author.is_empty() {
                        format!("{} {}", t.get("by"), e.author)
                    } else if !e.user {
                        "(Baba Is You)".into()
                    } else {
                        String::new()
                    };
                    let act = if upload {
                        Act::Ext(ExtAct::Upload(*i))
                    } else {
                        Act::Ext(ExtAct::PlayCustom(*i))
                    };
                    m.item(Item::new(name, act).value(value));
                }
                if pages > 1 {
                    let pg = |p: usize| {
                        Act::Ext(if upload {
                            ExtAct::UploadPage(p)
                        } else {
                            ExtAct::CustomPage(p)
                        })
                    };
                    m.gap(6.0);
                    m.row(vec![
                        Item::new("<", pg(page.saturating_sub(1))).enabled(page > 0),
                        Item::new(format!("{}/{}", page + 1, pages), Act::None),
                        Item::new(">", pg(page + 1)).enabled(page + 1 < pages),
                    ]);
                }
                m.gap(12.0);
                m.item(Item::new(t.get("return"), Act::Close));
            }
            ExtMenu::GetLevels => {
                m.top = 120.0;
                m.header.push(t.get("customlevels_play_get"));
                m.item(Item::new(
                    t.get("playlevels_get_code"),
                    open(ExtMenu::EnterCode {
                        text: String::new(),
                    }),
                ));
                m.item(Item::new(
                    t.get("playlevels_get_featured"),
                    Act::Ext(ExtAct::FeaturedPage(0)),
                ));
                m.item(Item::new(
                    t.get("customlevels_play_getlist"),
                    open(ExtMenu::History),
                ));
                m.gap(12.0);
                m.item(Item::new(t.get("return"), Act::Close));
            }
            ExtMenu::EnterCode { text } => {
                m.top = 60.0;
                m.header.push(t.get("namegiving_downloadcode"));
                m.header.push(format!("{text}_"));
                osk::add_rows(&mut m, osk::Charset::Code, false, "OK", &t.get("cancel"));
            }
            ExtMenu::Featured { page } => {
                m.top = 14.0;
                m.min_width = 560.0;
                m.header.push(t.get("playlevels_get_featured_colon"));
                let slot = &self.profile.slots[self.profile.last_slot()];
                if let Some(p) = self.ext.featured.as_ref().filter(|p| p.page == *page) {
                    for r in &p.entries {
                        let mut name = levelcodes::cut_name(r.name());
                        if store::won(slot, r.id()).won {
                            name.push_str(" \u{273D}");
                        }
                        let by = if r.author().is_empty() {
                            t.get("noauthor")
                        } else {
                            format!("{} {}", t.get("by"), r.author())
                        };
                        m.item(
                            Item::new(name, Act::Ext(ExtAct::Download(r.id().to_string())))
                                .value(by),
                        );
                    }
                    if p.pages > 1 {
                        let n = p.pages;
                        let go = |d: i64| {
                            let q = (*page as i64 + d).clamp(0, n as i64 - 1) as usize;
                            Act::Ext(ExtAct::FeaturedPage(q))
                        };
                        m.gap(4.0);
                        m.row(vec![
                            Item::new("<<", go(-5)).enabled(*page > 0),
                            Item::new("<", go(-1)).enabled(*page > 0),
                            Item::new(format!("{}/{}", page + 1, n), Act::None),
                            Item::new(">", go(1)).enabled(page + 1 < n),
                            Item::new(">>", go(5)).enabled(page + 1 < n),
                        ]);
                    }
                }
                m.gap(6.0);
                m.item(Item::new(t.get("return"), Act::Close));
            }
            ExtMenu::Wait => {
                m.top = 200.0;
                let list = self
                    .ext
                    .job
                    .as_ref()
                    .is_some_and(|j| j.what == JobKind::List);
                let up = self
                    .ext
                    .job
                    .as_ref()
                    .is_some_and(|j| j.what == JobKind::Upload);
                m.header.push(t.get(if list {
                    "playlevels_featured_wait"
                } else if up {
                    "editor_upload_uploading"
                } else {
                    "customlevels_get_wait"
                }));
            }
            ExtMenu::Found => {
                m.top = 110.0;
                m.header.push(t.get("customlevels_get"));
                if let Some(d) = &self.ext.downloaded {
                    let r = &d.record;
                    m.header.push(format!("$2,4{}", r.name()));
                    m.header.push(format!("{} {}", t.get("by"), r.author()));
                    if !r.subtitle().is_empty() {
                        m.header.push(r.subtitle().to_string());
                    }
                    m.header.push(format!("({})", d.code));
                }
                m.item(Item::new(
                    t.get("customlevels_get_save"),
                    Act::Ext(ExtAct::SaveDownloaded),
                ));
                m.item(Item::new(
                    t.get("customlevels_get_nosave"),
                    Act::Ext(ExtAct::PlayDownloaded),
                ));
                m.item(Item::new(t.get("customlevels_get_cancel"), Act::Close));
            }
            ExtMenu::Saved { stem } => {
                m.top = 150.0;
                m.header.push(t.get("customlevels_get_saved"));
                m.item(Item::new(
                    t.get("customlevels_get_play"),
                    Act::Ext(ExtAct::PlaySaved(stem.clone())),
                ));
                m.item(Item::new(
                    t.get("customlevels_get_levellist"),
                    open(ExtMenu::Custom { page: 0 }),
                ));
                m.item(Item::new(t.get("return"), Act::Close));
            }
            ExtMenu::Failed { list, err } => {
                m.top = 170.0;
                m.header.push(t.get(if *list {
                    "playlevels_featured_fail"
                } else {
                    "customlevels_get_fail"
                }));
                m.header
                    .push(format!("{} {err}", t.get("customlevels_get_fail_reason")));
                m.item(Item::new(t.get("return"), Act::Close));
            }
            ExtMenu::History => {
                m.top = 20.0;
                m.min_width = 300.0;
                let d = store::history(&self.profile.settings, "get_d");
                let u = store::history(&self.profile.settings, "get_u");
                m.header.push(t.get("customlevels_getlist_download"));
                m.item(Item::new(t.get("return"), Act::Close));
                let can = self.user_dir().is_some();
                let n = d.len().max(u.len());
                if n > 0 {
                    m.gap(6.0);
                }
                for i in 0..n {
                    let cell = |h: &[(String, String)]| match h.get(i) {
                        Some((c, name)) => Item::new(
                            format!("{}, {c}", levelcodes::cut_name(name)),
                            Act::Ext(ExtAct::Download(c.clone())),
                        )
                        .enabled(can),
                        None => Item::new("", Act::None).enabled(false),
                    };
                    m.row(vec![cell(&d), cell(&u)]);
                }
                m.footer.push(format!(
                    "{} {} / {} {}",
                    d.len(),
                    t.get("customlevels_getlist_download"),
                    u.len(),
                    t.get("customlevels_getlist_upload")
                ));
            }
            ExtMenu::ImportOffer | ExtMenu::Import => {
                let offer = matches!(id, ExtMenu::ImportOffer);
                m.top = 60.0;
                m.min_width = 300.0;
                m.header.push(if offer {
                    "Welcome! Your Baba Is You progress was found.".into()
                } else {
                    "Import your Baba Is You progress".into()
                });
                match &self.ext.found {
                    Some(f) => {
                        m.header.push(format!("{}", f.dir.display()));
                        for (i, s) in f.slots.iter().enumerate() {
                            m.header.push(match s {
                                Some(s) => format!(
                                    "{} {}: {} completed, {} \u{273D}  {} \u{273F}",
                                    t.get("slot"),
                                    i + 1,
                                    s.completed,
                                    s.stats.prize,
                                    s.stats.clears
                                ),
                                None => format!("{} {}: -", t.get("slot"), i + 1),
                            });
                        }
                        let ours: Vec<String> = (0..3)
                            .filter(|i| f.slots[*i].is_some() && !self.profile.slots[*i].is_empty())
                            .map(|i| (i + 1).to_string())
                            .collect();
                        if !ours.is_empty() && !offer {
                            m.header.push(format!(
                                "$2,2Replaces baba-port slot {} (the original is not changed)",
                                ours.join(", ")
                            ));
                        } else {
                            m.header.push("(the original's files are only read)".into());
                        }
                        if offer {
                            m.row(vec![
                                Item::new(t.get("yes"), Act::Ext(ExtAct::RunImport)),
                                Item::new(t.get("no"), Act::Ext(ExtAct::SkipImport)),
                            ]);
                        } else {
                            m.item(Item::new("Import", Act::Ext(ExtAct::RunImport)));
                            m.item(Item::new(t.get("return"), Act::Close));
                        }
                    }
                    None => {
                        m.header.push("No Baba Is You save was found in:".into());
                        for d in import::candidate_dirs().iter().take(4) {
                            m.header.push(format!("{}", d.display()));
                        }
                        m.item(Item::new(t.get("return"), Act::Close));
                    }
                }
                if offer {
                    m.escape = Act::Ext(ExtAct::SkipImport);
                }
            }
            ExtMenu::ImportDone => {
                m.top = 120.0;
                m.min_width = 200.0;
                if let Some(r) = &self.ext.report {
                    let slots: Vec<String> = r.slots.iter().map(|i| (i + 1).to_string()).collect();
                    m.header.push(format!(
                        "Imported: slot {}{}",
                        if slots.is_empty() {
                            "-".into()
                        } else {
                            slots.join(", ")
                        },
                        if r.settings {
                            ", settings and controls"
                        } else {
                            ""
                        }
                    ));
                    if r.levels_copied > 0 {
                        m.header
                            .push(format!("{} custom levels copied", r.levels_copied));
                    }
                    for n in r.notes.iter().take(5) {
                        m.header.push(format!("$0,1{n}"));
                    }
                }
                m.item(Item::new("OK", Act::Close));
            }
            ExtMenu::LuaNotice { pack } => {
                m.top = 130.0;
                for l in [
                    "This levelpack comes with Lua mods.",
                    "baba-port does not run Lua mods: the levels are",
                    "played with the standard rules, so some may not",
                    "work as their author intended.",
                ] {
                    m.header.push(l.into());
                }
                m.item(Item::new(
                    "Play anyway",
                    Act::Ext(ExtAct::StartPack(pack.clone())),
                ));
                m.item(Item::new(t.get("return"), Act::Close));
            }
            ExtMenu::UploadResult { ok, text } => {
                m.top = 170.0;
                m.header.push(t.get(if *ok {
                    "editor_upload_done"
                } else {
                    "editor_upload_failed"
                }));
                m.header.push(format!("$2,4{text}"));
                m.item(Item::new(t.get("return"), Act::Close));
            }
        }
        m
    }

    pub(super) fn ext_activate(&mut self, a: ExtAct) {
        match a {
            ExtAct::PlayCustom(i) => {
                if let Some(e) = self.custom_levels().get(i).cloned() {
                    let code = (!e.code.is_empty()).then(|| e.code.clone());
                    self.start_single(e.dir, &e.stem, code, false, false);
                }
            }
            ExtAct::CustomPage(p) => self.replace_top(ExtMenu::Custom { page: p }),
            ExtAct::UploadPage(p) => self.replace_top(ExtMenu::Upload { page: p }),
            ExtAct::SubmitCode => {
                let text = match self.menus.last() {
                    Some((MenuId::Ext(ExtMenu::EnterCode { text }), _)) => text.clone(),
                    _ => return,
                };
                match levelcodes::normalize_code(&text) {
                    Some(c) => {
                        self.menus.pop();
                        self.ext_activate(ExtAct::Download(c));
                    }
                    None => self.fail(false, &LcError::Invalid),
                }
            }
            ExtAct::Download(code) => {
                let Some(user) = self.user_dir() else {
                    return;
                };
                self.audio.sound("confirm");
                self.log.push(format!("download {code}"));
                self.ext.job = Some(Job::download(
                    Server::from_env(),
                    code,
                    store::temp_dir(&user),
                ));
                self.menus.push((ext(ExtMenu::Wait), (0, 0)));
            }
            ExtAct::FeaturedPage(page) => {
                if self.ext.featured.as_ref().is_some_and(|p| p.page == page) {
                    if matches!(
                        self.menus.last(),
                        Some((MenuId::Ext(ExtMenu::Featured { .. }), _))
                    ) {
                        self.replace_top(ExtMenu::Featured { page });
                    } else {
                        self.menus.push((ext(ExtMenu::Featured { page }), (0, 0)));
                    }
                    return;
                }
                let Some(user) = self.user_dir() else {
                    return;
                };
                if matches!(
                    self.menus.last(),
                    Some((MenuId::Ext(ExtMenu::Featured { .. }), _))
                ) {
                    self.menus.pop();
                }
                self.ext.job = Some(Job::featured(
                    Server::from_env(),
                    page,
                    store::thumbs_dir(&user),
                ));
                self.menus.push((ext(ExtMenu::Wait), (0, 0)));
            }
            ExtAct::SaveDownloaded => {
                let (Some(user), Some(d)) = (self.user_dir(), &self.ext.downloaded) else {
                    return;
                };
                match store::save_downloaded(&d.dir, &user) {
                    Ok(stem) => {
                        self.log.push(format!("saved {} as {stem}", d.code));
                        self.replace_top(ExtMenu::Saved { stem });
                    }
                    Err(e) => {
                        log::error!("saving level: {e}");
                        self.fail(false, &LcError::Server(e.to_string()));
                    }
                }
            }
            ExtAct::PlayDownloaded => {
                if let Some(d) = self.ext.downloaded.clone() {
                    self.start_single(d.dir, TEMP, Some(d.code), true, false);
                }
            }
            ExtAct::PlaySaved(stem) => {
                if let Some(u) = self.user_dir() {
                    let code = self.ext.downloaded.as_ref().map(|d| d.code.clone());
                    self.start_single(packs::user_levels_dir(&u), &stem, code, false, false);
                }
            }
            ExtAct::RunImport => self.run_import(),
            ExtAct::SkipImport => {
                self.log.push("import skipped".into());
                self.menus.clear();
                self.first_launch();
            }
            ExtAct::StartPack(id) => {
                packs::lua_notice_seen(&mut self.profile.settings, &id);
                self.profile.save_settings();
                self.menus.pop();
                self.log.push(format!("lua notice {id}"));
                let slot = self.profile.last_slot();
                self.audio.sound("confirm");
                self.start_trans(Then::StartWorld(slot, id));
            }
            ExtAct::LeaveSingle => {
                self.menus.clear();
                self.start_trans(Then::ToTitle);
            }
            ExtAct::Upload(i) => {
                if let Some(e) = self.custom_levels().get(i).cloned()
                    && UPLOAD
                {
                    self.start_single(e.dir, &e.stem, None, false, true);
                }
            }
        }
    }

    fn run_import(&mut self) {
        let Some(f) = self.ext.found.clone() else {
            return;
        };
        let langs: Vec<String> = self
            .texts
            .available
            .iter()
            .map(|(c, _)| c.clone())
            .collect();
        let lang_before = self.profile.settings.language();
        let fs_before = self
            .profile
            .settings
            .toggle(baba_progress::Toggle::Fullscreen);
        let mut r = import::import(&f.dir, &mut self.profile, &|c| langs.iter().any(|l| l == c));
        if let Some(u) = self.user_dir()
            && let Ok(to) = packs::ensure_user_levels(&u)
        {
            r.levels_copied =
                import::copy_levels(&self.data.join("Worlds").join(packs::LEVELS), &to);
        }
        self.profile.save_all();
        self.log.push(format!(
            "imported {} slots {:?} settings {}",
            f.dir.display(),
            r.slots,
            r.settings
        ));
        for n in &r.notes {
            log::info!("import: {n}");
        }
        self.ext.report = Some(r);
        // Apply what changed.
        let lang = self.profile.settings.language();
        if lang != lang_before {
            self.profile.settings.set_language(&lang_before);
            self.activate(Act::Language(lang), 0);
        }
        let fs = self
            .profile
            .settings
            .toggle(baba_progress::Toggle::Fullscreen);
        if fs != fs_before {
            self.fullscreen = Some(fs);
        }
        self.apply_settings();
        self.replace_top(ExtMenu::ImportDone);
    }

    fn start_single(
        &mut self,
        dir: PathBuf,
        stem: &str,
        code: Option<String>,
        temp: bool,
        upload: bool,
    ) {
        let slot = self.profile.last_slot();
        let Some(w) = WorldInfo::open_dir(&dir, packs::LEVELS) else {
            self.fail(false, &LcError::Invalid);
            return;
        };
        let mut env = Env {
            data_dir: &self.data,
            objects: &self.objects,
            atlas: &mut self.gfx.atlas,
        };
        let level = SingleLevel {
            code: code.clone(),
            temp,
        };
        match Play::single(&mut env, w, slot, stem, level) {
            Ok(mut p) => {
                self.log.append(&mut p.log);
                self.ext.back = self
                    .menus
                    .iter()
                    .filter(|(m, _)| {
                        !matches!(
                            m,
                            MenuId::Ext(
                                ExtMenu::Found
                                    | ExtMenu::Saved { .. }
                                    | ExtMenu::Wait
                                    | ExtMenu::EnterCode { .. }
                                    | ExtMenu::Failed { .. }
                            )
                        )
                    })
                    .cloned()
                    .collect();
                self.menus.clear();
                self.mode = Mode::Play(Box::new(p));
                self.ext.single = Some(Single {
                    code,
                    stem: stem.to_string(),
                    dir,
                    upload,
                    uploaded_ok: false,
                });
                self.level_music();
            }
            Err(e) => {
                log::error!("{e}");
                self.fail(false, &LcError::Invalid);
            }
        }
    }

    // ----------------------------------------------------------- upload

    #[cfg(feature = "upload")]
    pub(super) fn start_upload(&mut self, dir: &Path, stem: &str) {
        let read = |ext: &str| std::fs::read(dir.join(format!("{stem}.{ext}"))).unwrap_or_default();
        let (ld, l) = (read("ld"), read("l"));
        let ini = baba_formats::Ini::parse(&String::from_utf8_lossy(&ld));
        let g = |k: &str| ini.get("general", k).unwrap_or("").to_string();
        let (name, author, sub) = (g("name"), g("author"), g("subtitle"));
        let server = Server::from_env();
        self.ext.uploading = Some((dir.to_path_buf(), stem.to_string()));
        self.log.push(format!("upload {stem}"));
        self.ext.job = Some(Job::spawn(JobKind::Upload, move || {
            JobResult::Uploaded(server.upload(&sub, &author, &name, &ld, &l))
        }));
        self.menus.push((ext(ExtMenu::Wait), (0, 0)));
    }

    #[cfg(not(feature = "upload"))]
    pub(super) fn start_upload(&mut self, _dir: &Path, _stem: &str) {}

    fn upload_done(&mut self, r: Result<String, LcError>) {
        let (ok, text) = match r {
            Ok(code) => {
                if let Some((dir, stem)) = self.ext.uploading.take() {
                    let f = dir.join(format!("{stem}.ld"));
                    if let Ok(b) = std::fs::read(&f) {
                        let _ = std::fs::write(&f, store::with_levelcode(&b, &code));
                    }
                    let name = packs::levels_in(&dir, true)
                        .into_iter()
                        .find(|e| e.stem == stem)
                        .map(|e| e.name)
                        .unwrap_or_default();
                    store::push_history(&mut self.profile.settings, "get_u", &code, &name);
                    self.profile.save_settings();
                }
                self.log.push(format!("uploaded {code}"));
                (true, code)
            }
            Err(e) => (false, e.to_string()),
        };
        self.replace_top(ExtMenu::UploadResult { ok, text });
    }

    // ------------------------------------------------------------- draw

    fn image(&mut self, path: &Path) -> Option<RegionId> {
        if let Some(r) = self.ext.images.get(path) {
            return *r;
        }
        let r = RgbaImage::load_png(path)
            .ok()
            .filter(|i| {
                i.width + 2 < self.gfx.atlas.page_size()
                    && i.height + 2 < self.gfx.atlas.page_size()
            })
            .map(|i| self.gfx.atlas.insert(&i));
        self.ext.images.insert(path.to_path_buf(), r);
        r
    }

    /// Extra drawing over an M9 menu: thumbnail and tagline of the
    /// selected featured / custom level.
    pub(super) fn ext_draw(&mut self, ui: &mut Ui, id: &ExtMenu, sel: (usize, usize)) {
        let (thumb, lines) = match id {
            ExtMenu::Featured { page } => {
                let Some(p) = self.ext.featured.as_ref().filter(|p| p.page == *page) else {
                    return;
                };
                let Some(r) = p.entries.get(sel.0) else {
                    return;
                };
                let mut lines = vec![format!("$2,4{}", r.name()), r.id().to_string()];
                if !r.subtitle().is_empty() {
                    lines.push(r.subtitle().to_string());
                }
                lines.extend(wrap(&levelcodes::cut_desc(r.desc()), 30, 3));
                let thumb = self
                    .user_dir()
                    .map(|u| store::thumbs_dir(&u).join(format!("{}.png", r.id())));
                (thumb, lines)
            }
            ExtMenu::Custom { page } => {
                let all = self.custom_levels();
                let Some(e) = all.get(page * PER_PAGE + sel.0) else {
                    return;
                };
                let mut lines = Vec::new();
                if !e.subtitle.is_empty() {
                    lines.push(e.subtitle.clone());
                }
                if !e.author.is_empty() {
                    lines.push(format!("{} {}", self.texts.get("by"), e.author));
                }
                (Some(e.dir.join(format!("{}.png", e.stem))), lines)
            }
            _ => return,
        };
        let y0 = 345.0;
        let region = thumb.as_deref().and_then(|p| self.image(p));
        let g = &mut self.gfx;
        if let Some(r) = region {
            ui.sprite(r, 150.0, y0, 216.0, 112.0, [255; 4]);
        }
        let white = g.pal(0, 3);
        for (i, l) in lines.iter().enumerate() {
            ui.text(
                g,
                Some(&self.texts),
                l,
                390.0,
                y0 + 20.0 * i as f32,
                16.0,
                white,
                Align::Left,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn wraps_words() {
        assert_eq!(
            wrap("one two three four five", 9, 2),
            vec!["one two".to_string(), "three".to_string()]
        );
        assert!(wrap("", 9, 3).is_empty());
    }
}
