mod store;

use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
use std::path::Path;
use std::rc::Rc;

use dfui::taffy::{AlignItems, JustifyContent, Style};
use dfui::widgets::{Styled, button, column, icon, row, scroll, stateful, tabs, text, text_input, watch};
use dfui::{Color, Element, Event, EventCx, Font, Notifier, PaintCx, Rect, Role, Shadow, Shape, Widget};
use store::Sync;

const TABS: [&str; 3] = ["Swipe", "Names", "Agreed"];
const GREEN: Color = Color::hex(0x2e9d5b);
const RED: Color = Color::hex(0xd0453a);
/// Fraction of the card's width a drag must pass to count as a swipe.
const SWIPE_AT: f32 = 0.3;

struct Ui {
    sync: Sync,
    n: Notifier,
    up: Shape,
    down: Shape,
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
    });
    store::start(ui.sync.clone(), ui.n.clone());
    move || view(&ui)
}

fn view(ui: &Rc<Ui>) -> Element {
    let ui = ui.clone();
    stateful(move |&tab: &usize, h| {
        let page_ui = ui.clone();
        column([
            tabs(TABS, tab)
                .on_change(move |cx, i| cx.update(h, move |t: &mut usize| *t = i))
                .into(),
            watch(&ui.n, move || page(tab, &page_ui)).grow(1.0).into(),
        ])
        .fill()
    })
    .fill()
    .into()
}

fn page(tab: usize, ui: &Rc<Ui>) -> Element {
    let sh = ui.sync.lock().unwrap();
    let status = if sh.peer { "Partner connected" } else { "Looking for partner…" };
    let body = match tab {
        0 => swipe_page(sh.store.pending().first().cloned(), ui),
        1 => names_page(&sh.store, ui),
        _ => agreed_page(sh.store.agreed()),
    };
    column([
        text(status).size(13.0).color(Role::Muted).into(),
        column([body]).grow(1.0).into(),
    ])
    .gap(12.0)
    .padding(16.0)
    .grow(1.0)
    .into()
}

fn swipe_page(next: Option<String>, ui: &Rc<Ui>) -> Element {
    let Some(name) = next else {
        return centered(text("No names left to swipe. Add some!").color(Role::Muted).into());
    };
    // A new name gets a fresh card, so a finished drag doesn't carry over.
    let key = BuildHasherDefault::<DefaultHasher>::default().hash_one(&name);
    let ui = ui.clone();
    Element::new(Card {
        name: name.clone(),
        on_swipe: Box::new(move |_, yes| ui.change(|s| s.vote(&name, Some(yes)))),
    })
    .key(key)
}

/// Every name, with your swipe on it; tapping a swiped name puts it back in the deck.
fn names_page(store: &store::Store, ui: &Rc<Ui>) -> Element {
    let add_ui = ui.clone();
    let add = stateful(move |draft: &String, h| {
        let (ui, value) = (add_ui.clone(), draft.clone());
        row([
            text_input(draft.clone())
                .placeholder("A name")
                .on_change(move |cx, v| cx.update(h, |d: &mut String| *d = v))
                .grow(1.0)
                .into(),
            button(text("Add").color(Color::WHITE))
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
        let mark: Element = match mine {
            Some(true) => icon(&ui.up).color(GREEN).into(),
            Some(false) => icon(&ui.down).color(RED).into(),
            None => text("").into(),
        };
        let (ui, name) = (ui.clone(), name.clone());
        button(
            row([text(name.clone()).size(18.0).color(Role::Ink).grow(1.0).shrink(1.0).into(), mark])
                .align(AlignItems::CENTER)
                .grow(1.0),
        )
        .flat()
        .color(Role::Surface)
        .on_click(move |_| {
            if mine.is_some() {
                ui.change(|s| s.vote(&name, None));
            }
        })
        .into()
    });
    // ponytail: rebuilds every row on each change; switch to `list` if lists reach thousands.
    column([add.into(), scroll(column(rows).gap(2.0)).grow(1.0).into()])
        .gap(12.0)
        .grow(1.0)
        .into()
}

fn agreed_page(names: Vec<String>) -> Element {
    if names.is_empty() {
        return centered(text("No names you both like yet").color(Role::Muted).into());
    }
    let rows = names.into_iter().map(|name| {
        row([text(name).size(18.0).into()])
            .padding_xy(12.0, 10.0)
            .bg(Role::Surface)
            .radius(8.0)
            .into()
    });
    scroll(column(rows).gap(6.0)).grow(1.0).into()
}

fn centered(child: Element) -> Element {
    column([child]).align(AlignItems::CENTER).justify(JustifyContent::CENTER).grow(1.0).into()
}

type OnSwipe = Box<dyn Fn(&mut EventCx, bool)>;

/// A name card that follows the finger horizontally; released past
/// `SWIPE_AT` of its width it reports right (true) or left (false).
struct Card {
    name: String,
    on_swipe: OnSwipe,
}

#[derive(Default)]
struct Drag {
    start: Option<f32>,
    dx: f32,
}

impl Widget for Card {
    fn style(&self) -> Style {
        Style { flex_grow: 1.0, ..Style::default() }
    }

    fn paint(&self, cx: &mut PaintCx) {
        let dx = cx.state.get::<Drag>().dx;
        let r = Rect { x: cx.rect.x + dx, ..cx.rect };
        let t = (dx / (r.w * SWIPE_AT)).clamp(-1.0, 1.0);
        let tint = if t > 0.0 { GREEN } else { RED };
        let sh = Shadow { color: Color::BLACK.alpha(0.15), offset: (0.0, 4.0), blur: 16.0 };
        cx.shadow(r, 20.0, sh);
        cx.fill(r, 20.0, cx.theme.surface);
        cx.fill(r, 20.0, tint.alpha(t.abs() * 0.35));
        let size = 40.0;
        let layout = cx.measure(&self.name, Font::BOLD, size, Some(r.w - 32.0));
        let origin = (r.x + (r.w - layout.width) / 2.0, r.y + (r.h - layout.height) / 2.0);
        let ink = cx.theme.ink;
        cx.text(&self.name, Font::BOLD, size, ink, Some(r.w - 32.0), origin);
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
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    let dir = app.internal_data_path().unwrap_or_else(|| ".".into());
    dfui::run_android(self::app(&dir.join("names.txt")), app);
}
