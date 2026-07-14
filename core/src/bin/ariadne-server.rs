use ariadne::commands::{self, AriadneAppState};
use ariadne::rest::{run_rest_server, RestServerConfig};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = ServerArgs::parse(std::env::args().skip(1))?;
    std::env::set_var("ARIADNE_PROJECT_ROOT", &args.project_root);
    if let Some(app_state_root) = &args.app_state_root {
        std::env::set_var("ARIADNE_APP_STATE_ROOT", app_state_root);
    }
    let state = AriadneAppState::default_for_process();
    commands::get_current_project(&state)?;
    let config = RestServerConfig {
        bind: args.bind,
        bearer_token: args.token,
    };
    eprintln!(
        "ariadne-server listening on {} for project {}",
        config.bind, args.project_root
    );
    run_rest_server(state, config).map_err(|error| error.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerArgs {
    project_root: String,
    app_state_root: Option<String>,
    bind: String,
    token: String,
}

impl ServerArgs {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut project_root = None;
        let mut app_state_root = None;
        let mut bind = "127.0.0.1:4817".to_owned();
        let mut token = std::env::var("ARIADNE_REST_TOKEN").unwrap_or_default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--project" => project_root = Some(next_value(&mut args, "--project")?),
                "--app-state" => app_state_root = Some(next_value(&mut args, "--app-state")?),
                "--bind" => bind = next_value(&mut args, "--bind")?,
                "--token" => token = next_value(&mut args, "--token")?,
                "--help" | "-h" => return Err(usage()),
                other => return Err(format!("unsupported argument: {other}\n{}", usage())),
            }
        }
        let project_root = project_root.ok_or_else(usage)?;
        if token.trim().is_empty() {
            return Err("ARIADNE_REST_TOKEN or --token is required".to_owned());
        }
        Ok(Self {
            project_root,
            app_state_root,
            bind,
            token,
        })
    }
}

fn next_value(
    args: &mut impl Iterator<Item = String>,
    name: &'static str,
) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{name} requires a value\n{}", usage()))
}

fn usage() -> String {
    "usage: ariadne-server --project <path> [--app-state <path>] [--bind 127.0.0.1:4817] [--token <token>|ARIADNE_REST_TOKEN]".to_owned()
}
