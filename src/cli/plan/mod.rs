// src/cli/plan/mod.rs — Plan subcommand module and dispatcher (AN-018)

pub mod list;
pub mod new;
pub mod templates;
pub mod validate;

/// Entry point for `anchor plan new` — delegates to the wizard or template path.
pub fn run_new(output: Option<&str>, template: Option<&str>) -> i32 {
    new::run(output, template)
}

/// Entry point for `anchor plan list` — lists available templates.
pub fn run_list() -> i32 {
    list::run()
}

/// Entry point for `anchor plan validate` — validates a plan file without executing.
pub fn run_validate(plan_path: &str) -> i32 {
    validate::run(plan_path)
}
