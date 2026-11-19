/// Every text the UI shows, in one language.
pub struct Tr {
    pub swipe: &'static str,
    pub agreed: &'static str,
    pub names: &'static str,
    pub partner_on: &'static str,
    pub partner_off: &'static str,
    pub empty_deck: &'static str,
    pub placeholder: &'static str,
    pub add: &'static str,
    pub reset_score_title: &'static str,
    pub reset_score_body: fn(&str) -> String,
    pub reset: &'static str,
    pub cancel: &'static str,
    pub both_here: &'static str,
    pub need_both: &'static str,
    pub reveal: &'static str,
    pub no_agreed: &'static str,
    pub language: &'static str,
    pub reset_all: &'static str,
    pub reset_all_title: &'static str,
    pub reset_all_body: &'static str,
}

pub const LANGUAGES: [&str; 2] = ["English", "Deutsch"];

pub fn get(german: bool) -> &'static Tr {
    if german { &DE } else { &EN }
}

const EN: Tr = Tr {
    swipe: "Swipe",
    agreed: "Agreed",
    names: "Names",
    partner_on: "Partner connected",
    partner_off: "Looking for partner…",
    empty_deck: "No names yet. Add some under Names in the menu.",
    placeholder: "A name",
    add: "Add",
    reset_score_title: "Reset score?",
    reset_score_body: |n| format!("{n} goes back to score 0 and into the deck as new."),
    reset: "Reset",
    cancel: "Cancel",
    both_here: "You're both here.",
    need_both: "Reveal needs both partners on the same Wi-Fi.",
    reveal: "Reveal!",
    no_agreed: "No names you both like yet",
    language: "Language",
    reset_all: "Reset everything",
    reset_all_title: "Reset everything?",
    reset_all_body: "Deletes all swipes, scores and added names, on both phones.",
};

const DE: Tr = Tr {
    swipe: "Swipen",
    agreed: "Einig",
    names: "Namen",
    partner_on: "Partner verbunden",
    partner_off: "Suche Partner…",
    empty_deck: "Noch keine Namen. Füge im Menü unter Namen welche hinzu.",
    placeholder: "Ein Name",
    add: "Hinzufügen",
    reset_score_title: "Punkte zurücksetzen?",
    reset_score_body: |n| format!("{n} geht zurück auf 0 Punkte und kommt neu in den Stapel."),
    reset: "Zurücksetzen",
    cancel: "Abbrechen",
    both_here: "Ihr seid beide da.",
    need_both: "Zum Aufdecken müsst ihr beide im selben WLAN sein.",
    reveal: "Aufdecken!",
    no_agreed: "Noch keine Namen, die euch beiden gefallen",
    language: "Sprache",
    reset_all: "Alles zurücksetzen",
    reset_all_title: "Alles zurücksetzen?",
    reset_all_body: "Löscht alle Swipes, Punkte und hinzugefügten Namen, auf beiden Handys.",
};
