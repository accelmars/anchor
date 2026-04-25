// src/cli/plan/mod.rs — Plan subcommand module and dispatcher (AN-018)

pub mod list;
pub mod new;
pub mod templates;

/// Entry point for `anchor plan new` — delegates to the wizard.
pub fn run_new(output: Option<&str>) -> i32 {
    new::run(output)
}

/// Entry point for `anchor plan list` — lists available templates.
pub fn run_list() -> i32 {
    list::run()
}
