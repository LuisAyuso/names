mod store;
mod tr;
mod wallpaper;

use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
use std::path::Path;
use std::rc::Rc;

use dfui::taffy::{AlignItems, JustifyContent, Style};
use dfui::widgets::{
    Overlays, Styled, button, column, dialog, drawer, icon, icon_button, on_back, overlays, row,
    scroll, segmented, stateful, tabs, text, text_input, watch,
};
use dfui::{
    Color, Element, Event, EventCx, Font, Handle, Notifier, PaintCx, Rect, Role, Shadow, Shape,
    Widget,
};
use store::Sync;
use tr::Tr;
use wallpaper::{INK, Wallpaper};

const GREEN: Color = Color::hex(0x2e9d5b);
const RED: Color = Color::hex(0xd0453a);
const PINK: Color = Color::hex(0xf7c6da);
const BLUE: Color = Color::hex(0xc4dbf7);
/// Fraction of the card's width a drag must pass to count as a swipe.
const SWIPE_AT: f32 = 0.3;

struct Ui {
    sync: Sync,
    n: Notifier,
    up: Shape,
    down: Shape,
    menu: Shape,
    back: Shape,
}

impl Ui {
    /// Applies `f` to the store, then saves and rebuilds; the next broadcast carries it.
    fn change(&self, f: impl FnOnce(&mut store::Store)) {
        let mut sh = self.sync.lock().unwrap();
        f(&mut sh.store);
        sh.save();
        self.n.notify();
    }
}

pub fn app(data: &Path) -> impl dfui::App {
    let shape = |b: &[u8]| Shape::from_bytes(b).expect("bundled icon");
    let ui = Rc::new(Ui {
        sync: store::load(data),
        n: Notifier::new(),
        up: shape(include_bytes!("../assets/icons/thumb_up.path")),
        down: shape(include_bytes!("../assets/icons/thumb_down.path")),
        menu: shape(include_bytes!("../assets/icons/menu.path")),
        back: shape(include_bytes!("../assets/icons/arrow-left.path")),
    });
    store::start(ui.sync.clone(), ui.n.clone());
    move || view(&ui)
}

/// Where the user is: a main tab (0 Swipe, 1 Agreed) or the Names screen
/// over it, and whether Agreed was revealed; any navigation hides it again.
#[derive(Clone, Copy, Default)]
struct Nav {
    tab: usize,
    names: bool,
    shown: bool,
}

fn view(ui: &Rc<Ui>) -> Element {
    let ui = ui.clone();
    overlays(move |ov| {
        let ui = ui.clone();
        stateful(move |&nav: &Nav, h| {
            let ui = ui.clone();
            watch(&ui.n.clone(), move || screen(nav, h, ov, &ui)).fill()
        })
        .fill()
    })
    .fill()
    .into()
}

fn go(h: Handle<Nav>, to: Nav) -> impl Fn(&mut EventCx) + Clone + 'static {
    move |cx| cx.update(h, move |n: &mut Nav| *n = to)
}

fn screen(nav: Nav, h: Handle<Nav>, ov: Handle<Overlays>, ui: &Rc<Ui>) -> Element {
    let sh = ui.sync.lock().unwrap();
    let t = tr::get(sh.store.german);
    let main = Nav {
        names: false,
        shown: false,
        ..nav
    };
    let header: Element = if nav.names {
        let back = go(h, main);
        row([
            on_back(back.clone()).into(),
            icon_button(&ui.back).on_click(back).into(),
            text(t.names).size(20.0).into(),
        ])
        .gap(8.0)
        .align(AlignItems::CENTER)
        .into()
    } else {
        let menu_ui = ui.clone();
        row([
            icon_button(&ui.menu)
                .on_click(move |cx| open_menu(cx, ov, h, nav, &menu_ui))
                .into(),
            tabs([t.swipe, t.agreed], nav.tab)
                .on_change(move |cx, i| {
                    go(
                        h,
                        Nav {
                            tab: i,
                            ..Nav::default()
                        },
                    )(cx)
                })
                .grow(1.0)
                .into(),
        ])
        .align(AlignItems::CENTER)
        .into()
    };
    let body = if nav.names {
        names_page(&sh.store, t, ov, ui)
    } else if nav.tab == 0 {
        swipe_page(sh.store.next(), t, ui)
    } else {
        let reveal = go(h, Nav { shown: true, ..nav });
        agreed_page(sh.store.agreed(), sh.peer, nav.shown, t, reveal)
    };
    let status = if sh.peer { t.partner_on } else { t.partner_off };
    column([
        header,
        column([
            text(status).size(13.0).color(Role::Muted).into(),
            column([body]).fill().into(),
        ])
        .gap(12.0)
        .padding(16.0)
        .fill()
        .into(),
    ])
    .fill()
    .into()
}

/// Side drawer with Names, the language switch and the full reset.
fn open_menu(cx: &mut EventCx, ov: Handle<Overlays>, h: Handle<Nav>, nav: Nav, ui: &Rc<Ui>) {
    let ui = ui.clone();
    ov.show(cx, move |me| {
        let ui = ui.clone();
        let n = ui.n.clone();
        drawer(watch(&n, move || {
            let german = ui.sync.lock().unwrap().store.german;
            let t = tr::get(german);
            let lang_ui = ui.clone();
            let reset_ui = ui.clone();
            let open_names = go(
                h,
                Nav {
                    names: true,
                    shown: false,
                    ..nav
                },
            );
            column([
                button(text(t.names).size(18.0).color(Role::Ink))
                    .flat()
                    .color(Role::Surface)
                    .on_click(move |cx| {
                        me.close(cx);
                        open_names(cx);
                    })
                    .into(),
                text(t.language).size(13.0).color(Role::Muted).into(),
                segmented(tr::LANGUAGES, german as usize)
                    .on_change(move |_, i| lang_ui.change(|s| s.german = i == 1))
                    .into(),
                button(text(t.reset_all).color(Color::WHITE))
                    .color(RED)
                    .on_click(move |cx| {
                        me.close(cx);
                        let ui = reset_ui.clone();
                        confirm(
                            cx,
                            ov,
                            t,
                            t.reset_all_title,
                            t.reset_all_body.into(),
                            t.reset_all,
                            move |_| ui.change(|s| s.reset()),
                        );
                    })
                    .into(),
            ])
            .gap(16.0)
            .padding(8.0)
        }))
    });
}

/// A dialog with `title`, `body`, a translated Cancel and `ok`, which runs `on_ok`.
fn confirm(
    cx: &mut EventCx,
    ov: Handle<Overlays>,
    t: &'static Tr,
    title: &'static str,
    body: String,
    ok: &'static str,
    on_ok: impl Fn(&mut EventCx) + 'static,
) {
    let on_ok = Rc::new(on_ok);
    ov.show(cx, move |me| {
        let on_ok = on_ok.clone();
        dialog(
            column([
                text(title).size(20.0).into(),
                text(body.clone()).size(14.0).color(Role::Muted).into(),
                row([
                    button(text(t.cancel).color(Role::Ink))
                        .color(Role::Surface)
                        .flat()
                        .on_click(move |cx| me.close(cx))
                        .into(),
                    button(text(ok).color(Color::WHITE))
                        .flat()
                        .on_click(move |cx| {
                            me.close(cx);
                            on_ok(cx);
                        })
                        .into(),
                ])
                .gap(8.0)
                .justify(JustifyContent::FLEX_END)
                .into(),
            ])
            .gap(12.0),
        )
    });
}

fn swipe_page(next: Option<String>, t: &Tr, ui: &Rc<Ui>) -> Element {
    let Some(name) = next else {
        return centered(text(t.empty_deck).color(Role::Muted).into());
    };
    // A new name gets a fresh card, so a finished drag doesn't carry over.
    let key = BuildHasherDefault::<DefaultHasher>::default().hash_one(&name);
    let ui = ui.clone();
    Element::new(Card {
        name: name.clone(),
        wall: Wallpaper::new(key),
        on_swipe: Box::new(move |_, yes| ui.change(|s| s.vote(&name, Some(yes)))),
    })
    .key(key)
}

/// Every name with your score; tapping a swiped one asks, then puts it back in
/// the deck; swiping one left asks, then removes it for good.
fn names_page(store: &store::Store, t: &'static Tr, ov: Handle<Overlays>, ui: &Rc<Ui>) -> Element {
    let add_ui = ui.clone();
    let add = stateful(move |draft: &String, h| {
        let (ui, value) = (add_ui.clone(), draft.clone());
        row([
            text_input(draft.clone())
                .placeholder(t.placeholder)
                .on_change(move |cx, v| cx.update(h, |d: &mut String| *d = v))
                .grow(1.0)
                .into(),
            button(text(t.add).color(Color::WHITE))
                .enabled(!draft.trim().is_empty())
                .on_click(move |cx| {
                    ui.change(|s| s.add(&value));
                    cx.update(h, |d: &mut String| d.clear());
                })
                .into(),
        ])
        .gap(8.0)
        .align(AlignItems::CENTER)
    });
    let rows = store.names.iter().map(|name| {
        let mine = store.mine(name);
        let score = store.score(store.me, name);
        let (shape, color) = if score > 0 {
            (&ui.up, GREEN)
        } else {
            (&ui.down, RED)
        };
        let mark: Element = match mine {
            Some(_) => row([
                text(format!("{score:+}")).size(16.0).color(color).into(),
                icon(shape).color(color).into(),
            ])
            .gap(6.0)
            .align(AlignItems::CENTER)
            .into(),
            None => text("").into(),
        };
        let (ui, name) = (ui.clone(), name.clone());
        let (remove_ui, remove_name) = (ui.clone(), name.clone());
        let row = button(
            row([
                text(name.clone())
                    .size(18.0)
                    .color(Role::Ink)
                    .grow(1.0)
                    .shrink(1.0)
                    .into(),
                mark,
            ])
            .align(AlignItems::CENTER)
            .grow(1.0),
        )
        .flat()
        .color(Role::Surface)
        .on_click(move |cx| {
            if mine.is_some() {
                let (ui, name) = (ui.clone(), name.clone());
                let body = (t.reset_score_body)(&name);
                confirm(cx, ov, t, t.reset_score_title, body, t.reset, move |_| {
                    ui.change(|s| s.vote(&name, None))
                });
            }
        });
        Element::new(SwipeAway {
            row: [row.into()],
            label: t.remove,
            on_remove: Box::new(move |cx| {
                let (ui, name) = (remove_ui.clone(), remove_name.clone());
                let body = (t.remove_body)(&name);
                confirm(cx, ov, t, t.remove_title, body, t.remove, move |_| {
                    ui.change(|s| s.remove(&name))
                });
            }),
        })
    });
    // ponytail: rebuilds every row on each change; switch to `list` if lists reach thousands.
    column([add.into(), scroll(column(rows).gap(2.0)).fill().into()])
        .gap(12.0)
        .fill()
        .into()
}

/// Hidden until both partners are connected and Reveal! is tapped.
fn agreed_page(
    names: Vec<(String, i32, i32)>,
    peer: bool,
    shown: bool,
    t: &Tr,
    reveal: impl Fn(&mut EventCx) + 'static,
) -> Element {
    if !shown {
        return column([
            text(if peer { t.both_here } else { t.need_both })
                .color(Role::Muted)
                .into(),
            button(text(t.reveal).size(22.0).color(Color::WHITE))
                .enabled(peer)
                .on_click(reveal)
                .into(),
        ])
        .gap(16.0)
        .align(AlignItems::CENTER)
        .justify(JustifyContent::CENTER)
        .fill()
        .into();
    }
    if names.is_empty() {
        return centered(text(t.no_agreed).color(Role::Muted).into());
    }
    let rows = names.iter().map(|(name, pink, blue)| {
        Element::new(ScoreRow {
            name: name.clone(),
            pink: *pink,
            blue: *blue,
        })
    });
    scroll(column(rows).gap(6.0)).fill().into()
}

fn centered(child: Element) -> Element {
    column([child])
        .align(AlignItems::CENTER)
        .justify(JustifyContent::CENTER)
        .fill()
        .into()
}

/// An agreed name over a pink and a blue band, each as wide as that
/// partner's share of the combined score.
struct ScoreRow {
    name: String,
    pink: i32,
    blue: i32,
}

impl Widget for ScoreRow {
    fn style(&self) -> Style {
        Style {
            size: dfui::taffy::Size {
                width: dfui::taffy::Dimension::percent(1.0),
                height: dfui::taffy::Dimension::length(52.0),
            },
            flex_shrink: 0.0,
            ..Style::default()
        }
    }

    fn paint(&self, cx: &mut PaintCx) {
        let r = cx.rect;
        let total = (self.pink + self.blue) as f32;
        let split = Rect {
            w: r.w * self.pink as f32 / total,
            ..r
        };
        cx.fill(r, 10.0, BLUE);
        cx.with_clip(split, |cx| cx.fill(r, 10.0, PINK));
        let ink = cx.theme.ink;
        let size = 20.0;
        let h = cx.measure(&self.name, Font::BOLD, size, None).height;
        let y = r.y + (r.h - h) / 2.0;
        cx.text(&self.name, Font::BOLD, size, ink, None, (r.x + 16.0, y));
        let score = format!("{}", self.pink + self.blue);
        let w = cx.measure(&score, Font::DEFAULT, size, None).width;
        cx.text(
            &score,
            Font::DEFAULT,
            size,
            ink,
            None,
            (r.x + r.w - 16.0 - w, y),
        );
    }
}

type OnSwipe = Box<dyn Fn(&mut EventCx, bool)>;

/// Lets its row be dragged left over a red strip; released past `SWIPE_AT`
/// of its width it calls `on_remove`. Vertical drags go to the scroller.
struct SwipeAway {
    row: [Element; 1],
    label: &'static str,
    on_remove: Box<dyn Fn(&mut EventCx)>,
}

#[derive(Default)]
struct Slide {
    start: Option<(f32, f32)>,
    /// The drag turned out horizontal, so this row owns it.
    sideways: bool,
    dx: f32,
}

/// Movement before a drag counts as horizontal or vertical.
const SLOP: f32 = 8.0;

impl Widget for SwipeAway {
    fn children(&self) -> &[Element] {
        &self.row
    }

    fn scroll(
        &self,
        state: &mut dfui::State,
        _: dfui::Placement,
        _: dfui::Instant,
    ) -> Option<(f32, f32)> {
        Some((-state.get::<Slide>().dx, 0.0))
    }

    fn paint(&self, cx: &mut PaintCx) {
        let dx = cx.state.get::<Slide>().dx;
        if dx >= 0.0 {
            return;
        }
        let r = cx.rect;
        let past = -dx > r.w * SWIPE_AT;
        cx.fill(r, 0.0, RED.alpha(if past { 1.0 } else { 0.6 }));
        let size = 16.0;
        let layout = cx.measure(self.label, Font::BOLD, size, None);
        let origin = (
            r.x + r.w - 16.0 - layout.width,
            r.y + (r.h - layout.height) / 2.0,
        );
        cx.text(self.label, Font::BOLD, size, Color::WHITE, None, origin);
    }

    fn event(&self, ev: &Event, cx: &mut EventCx) {
        let w = cx.rect.w;
        let s = cx.state.get::<Slide>();
        match *ev {
            Event::PointerDown { x, y } => {
                *s = Slide {
                    start: Some((x, y)),
                    ..Slide::default()
                }
            }
            Event::PointerMove { x, y } => {
                let Some((sx, sy)) = s.start else { return };
                let (dx, dy) = (x - sx, y - sy);
                if !s.sideways {
                    if dx.abs() > SLOP && dx.abs() > dy.abs() {
                        s.sideways = true;
                    } else {
                        if dy.abs() > SLOP {
                            s.start = None;
                        }
                        return;
                    }
                }
                s.dx = dx.min(0.0);
                cx.capture();
                cx.request_redraw();
            }
            Event::PointerUp { .. } => {
                let remove = s.sideways && -s.dx > w * SWIPE_AT;
                *s = Slide::default();
                cx.request_redraw();
                if remove {
                    (self.on_remove)(cx);
                }
            }
            Event::Cancel => {
                *s = Slide::default();
                cx.request_redraw();
            }
            _ => {}
        }
    }
}

/// A name card that follows the finger horizontally; released past
/// `SWIPE_AT` of its width it reports right (true) or left (false).
struct Card {
    name: String,
    wall: Wallpaper,
    on_swipe: OnSwipe,
}

#[derive(Default)]
struct Drag {
    start: Option<f32>,
    dx: f32,
}

impl Widget for Card {
    fn style(&self) -> Style {
        Style {
            flex_grow: 1.0,
            ..Style::default()
        }
    }

    fn paint(&self, cx: &mut PaintCx) {
        let dx = cx.state.get::<Drag>().dx;
        let r = Rect {
            x: cx.rect.x + dx,
            ..cx.rect
        };
        let t = (dx / (r.w * SWIPE_AT)).clamp(-1.0, 1.0);
        let tint = if t > 0.0 { GREEN } else { RED };
        let sh = Shadow {
            color: Color::BLACK.alpha(0.15),
            offset: (0.0, 4.0),
            blur: 16.0,
        };
        cx.shadow(r, 20.0, sh);
        self.wall.paint(cx, r, 20.0);
        let (font, size) = (self.wall.font, 40.0 * self.wall.scale);
        let layout = cx.measure(&self.name, font, size, Some(r.w - 32.0));
        let origin = (
            r.x + (r.w - layout.width) / 2.0,
            r.y + (r.h - layout.height) / 2.0,
        );
        // A soft patch of the ground behind the name keeps motifs from crowding it.
        let pad = 16.0;
        let halo = Rect::new(
            origin.0 - pad,
            origin.1 - pad,
            layout.width + 2.0 * pad,
            layout.height + 2.0 * pad,
        );
        let glow = Shadow {
            color: self.wall.ground.alpha(0.9),
            offset: (0.0, 0.0),
            blur: 24.0,
        };
        cx.shadow(halo, glow.blur, glow);
        cx.fill(r, 20.0, tint.alpha(t.abs() * 0.35));
        cx.text(&self.name, font, size, INK, Some(r.w - 32.0), origin);
    }

    fn event(&self, ev: &Event, cx: &mut EventCx) {
        let w = cx.rect.w;
        let d = cx.state.get::<Drag>();
        match *ev {
            Event::PointerDown { x, .. } => d.start = Some(x),
            Event::PointerMove { x, .. } => {
                let Some(s) = d.start else { return };
                d.dx = x - s;
                cx.capture();
                cx.request_redraw();
            }
            Event::PointerUp { .. } => {
                let dx = std::mem::take(&mut d.dx);
                d.start = None;
                cx.request_redraw();
                if dx.abs() > w * SWIPE_AT {
                    (self.on_swipe)(cx, dx > 0.0);
                }
            }
            Event::Cancel => *d = Drag::default(),
            _ => {}
        }
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: dfui::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    let dir = app.internal_data_path().unwrap_or_else(|| ".".into());
    dfui::run_android(self::app(&dir.join("names.txt")), app);
}
