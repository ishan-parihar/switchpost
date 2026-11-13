// ─── AXI §8 + §10: Content-first home view ──────────────────────
// Shows live state when no args provided, following AXI principles.
// Lightweight — no DB access, just tool identity and command hints.

/// AXI §10: Get home directory path with ~ collapse
fn get_bin_path() -> String {
    std::env::current_exe()
        .map(|p| {
            let s = p.display().to_string();
            if let Ok(home) = std::env::var("HOME") {
                s.replacen(&home, "~", 1)
            } else {
                s
            }
        })
        .unwrap_or_else(|_| "switchpost".to_string())
}

/// AXI §8: Content-first home view — shows tool identity and command hints
pub fn handle_home() {
    let bin_path = get_bin_path();

    // AXI §10: Tool identity header
    println!("bin: {}", bin_path);
    println!(
        "description: Post to 25 social platforms from a single CLI with auto content-splitting"
    );
    println!();

    // Supported platforms (minimal schema — AXI §2)
    let platforms = [
        "x",
        "reddit",
        "linkedin",
        "facebook",
        "instagram",
        "bluesky",
        "mastodon",
        "youtube",
        "tiktok",
        "threads",
        "discord",
        "slack",
        "telegram",
        "whatsapp",
        "pinterest",
        "github",
        "wordpress",
        "hashnode",
        "medium",
        "devto",
        "skool",
        "linkedin-page",
        "instagram-standalone",
        "google_my_business",
    ];

    println!("platforms[{}]{{name}}:", platforms.len());
    for p in &platforms {
        println!("  {}", p);
    }
    println!();

    // AXI §9: Contextual disclosure — suggest next steps
    println!("help[6]:");
    println!("  Run `switchpost providers` to see connected accounts");
    println!("  Run `switchpost setup` for guided onboarding");
    println!("  Run `switchpost connect <provider>` to connect a platform");
    println!("  Run `switchpost post \"text\" --platforms x,linkedin` to post");
    println!("  Run `switchpost doctor` to check provider health");
    println!("  Run `switchpost mcp` to start the MCP server for AI agents");
}
