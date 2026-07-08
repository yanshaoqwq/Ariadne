fn main() {
    let result = ariadne::cli::run_cli(std::env::args().skip(1));
    print!("{}", result.stdout);
    std::process::exit(result.exit_code);
}
