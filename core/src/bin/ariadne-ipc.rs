fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None | Some("stdio") => {
            ariadne::ipc::run_json_line_stdio().map_err(|error| error.to_string())
        }
        Some("call") => {
            let method = args
                .next()
                .ok_or_else(|| "usage: ariadne-ipc call <method> [params-json]".to_owned())?;
            let params_json = args.next();
            if args.next().is_some() {
                return Err("usage: ariadne-ipc call <method> [params-json]".to_owned());
            }
            let response = ariadne::ipc::run_single_call(&method, params_json.as_deref())?;
            println!(
                "{}",
                serde_json::to_string(&response)
                    .map_err(|error| format!("failed to serialize ipc response: {error}"))?
            );
            if response.ok {
                Ok(())
            } else {
                Err(response
                    .error
                    .unwrap_or_else(|| "ipc call failed".to_owned()))
            }
        }
        Some("--help") | Some("-h") => {
            println!("usage: ariadne-ipc [stdio|call <method> [params-json]]");
            Ok(())
        }
        Some(other) => Err(format!("unsupported ariadne-ipc mode: {other}")),
    }
}
