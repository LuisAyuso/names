fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match &args[..] {
        [flag, path, name] if flag == "--headless" => names::headless(path.as_ref(), name),
        [path] => dfui::run(names::app(path.as_ref())),
        _ => dfui::run(names::app("names.txt".as_ref())),
    }
}
