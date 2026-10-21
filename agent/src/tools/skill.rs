use serde_json::Value;

use super::Ctx;
use crate::skills;

/// Load the full instructions of a discovered skill by name.
pub fn load(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("missing string field `name`")?;

    if ctx.skills.is_empty() {
        return Err("no skills are configured for this workspace".to_string());
    }

    let skill = ctx.skills.iter().find(|s| s.name == name).ok_or_else(|| {
        let names: Vec<&str> = ctx.skills.iter().map(|s| s.name.as_str()).collect();
        format!("unknown skill `{name}`; available: {}", names.join(", "))
    })?;

    let body = skills::load_body(&skill.path)?;
    Ok(format!(
        "Skill `{}` ({}):\n\n{}",
        skill.name, skill.description, body
    ))
}
