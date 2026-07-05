fn main() {
    if let Err(error) = ariadne::ipc::run_json_line_stdio() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
