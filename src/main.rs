fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "names.txt".into());
    dfui::run(names::app(path.as_ref()));
}
