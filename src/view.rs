use crate::db::User;

pub const SCRIPT: &str = r#"<script>(()=>{let lost=false;const c=()=>{const es=new EventSource("/__livereload");es.addEventListener("reload",()=>location.reload());es.onopen=()=>{if(lost)location.reload()};es.onerror=()=>{lost=true;es.close();setTimeout(c,500)}};c()})()</script>"#;

pub fn render(
    name: &str,
    dev: bool,
    user: Option<&User>,
    auth_enabled: bool,
) -> Result<String, minijinja::Error> {
    let mut env = minijinja::Environment::new();
    env.set_loader(minijinja::path_loader("templates"));
    let ctx = match user {
        Some(u) => minijinja::context! { user => u, auth_enabled => auth_enabled },
        None => minijinja::context! { auth_enabled => auth_enabled },
    };
    let mut html = env.get_template(name)?.render(ctx)?;
    if dev {
        match html.rfind("</body>") {
            Some(i) => html.insert_str(i, SCRIPT),
            None => html.push_str(SCRIPT),
        }
    }
    Ok(html)
}
