// ─── Agent-surface meta audit (D1 + D2 from OUTLINE-2026-10-19) ─────────
// Pure source/doc parsing — no DB, no network. Guards the two drift
// classes that cost agents context:
//   D1: the documented tool count vs the registered #[tool(] count.
//   D2: schema shape — every tool carries a non-empty, unique,
//       size-bounded description and a provider-prefixed name.

use std::path::PathBuf;

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// (name, description) for every `#[tool(description = "…")]` + the fn that
/// follows it. Fails loudly on an unparsed `#[tool(` line, and asserts the
/// parsed count equals the raw `#[tool(` count so the check can never
/// silently drop a tool.
fn parse_tools() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut raw_attrs = 0;
    for entry in std::fs::read_dir(repo_path("src/mcp")).expect("read src/mcp") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read source");
        let lines: Vec<&str> = src.lines().collect();
        raw_attrs += lines
            .iter()
            .filter(|l| l.trim().starts_with("#[tool("))
            .count();
        let mut i = 0;
        while i < lines.len() {
            let t = lines[i].trim();
            if !t.starts_with("#[tool(") {
                i += 1;
                continue;
            }
            // Two forms in-tree: single-line and `#[tool(` … `)]` spanning
            // lines. Collect the whole macro, then pull the description.
            let mut end = i;
            let mut macro_text = t.to_string();
            while !lines[end].trim().ends_with(")]") {
                end += 1;
                let l = lines
                    .get(end)
                    .unwrap_or_else(|| panic!("unterminated #[tool(] in {path:?}"));
                macro_text.push(' ');
                macro_text.push_str(l.trim());
            }
            let desc = extract_description(&macro_text).unwrap_or_else(|| {
                panic!("unparsed #[tool(] (extend parser) in {path:?}: {macro_text}")
            });
            // Methods are `pub async fn` or plain `async fn`. Stop at the
            // first fn line and never step over another `#[tool(`.
            let mut j = end + 1;
            let name = loop {
                let l = lines
                    .get(j)
                    .unwrap_or_else(|| panic!("no fn after #[tool] in {path:?}"))
                    .trim();
                assert!(
                    !l.starts_with("#[tool("),
                    "#[tool( with no fn before line {} in {path:?}",
                    j + 1
                );
                if let Some(name) = fn_name(l) {
                    break name;
                }
                j += 1;
            };
            out.push((name, desc));
            i = j + 1;
        }
    }
    assert_eq!(out.len(), raw_attrs, "parser dropped #[tool( attributes");
    out
}

/// Name of a `[pub] [async] fn <name>` declaration line, if this is one.
fn fn_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("pub ").unwrap_or(line);
    let rest = rest.strip_prefix("async ").unwrap_or(rest);
    let rest = rest.strip_prefix("fn ")?;
    Some(
        rest.split(['(', '<'])
            .next()
            .unwrap_or("")
            .trim()
            .to_string(),
    )
}

/// `description = "…"` with the quotes as delimiters. Description text
/// containing an escaped quote is not currently in-tree; the None path
/// panics at the call site so the check can never silently pass.
fn extract_description(macro_text: &str) -> Option<String> {
    let start = macro_text.find("description = \"")? + "description = \"".len();
    let rest = &macro_text[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[test]
fn d1_documented_tool_counts_match_registry() {
    let tools = parse_tools();
    let count = tools.len();
    assert!(count > 0, "no tools parsed");
    let agents = std::fs::read_to_string(repo_path("AGENTS.md")).unwrap();
    assert!(
        agents.contains(&format!("{count} MCP tools")),
        "AGENTS.md must state the true MCP tool count ({count} MCP tools)"
    );
    let providers = std::fs::read_to_string(repo_path(
        "skills/switchpost-agent/references/mcp-catalogue.md",
    ))
    .expect("read mcp-catalogue.md");
    // The catalogue's title states the total: "# MCP catalogue — 389 tools by category".
    assert!(
        providers.contains(&format!("{count} tools")),
        "mcp-catalogue.md must state the true tool count ({count} tools)"
    );
    let readme = std::fs::read_to_string(repo_path("README.md")).unwrap();
    assert!(
        readme.contains(&format!("{count} MCP tools")),
        "README.md must state the true MCP tool count ({count} MCP tools)"
    );
    assert!(
        !readme.contains("327 tools") && !readme.contains("327 MCP"),
        "README.md still states the stale 327-tool count"
    );
}

#[test]
fn d1_documented_provider_count_matches_the_registry() {
    // The provider count drifted in the PROSE for a month while
    // `src/social/tier.rs` ASSERTED the true numbers. An assertion on the
    // code cannot catch a sentence that describes it, so the doc needs its
    // own gate - and this is the gate that was missing.
    //
    // The count is the REGISTRY's, not the tier constants': Tier-3
    // (farcaster) is only registered when ENABLE_ARCHIVE_PROVIDERS is set,
    // so "registered by default" is Tier-1 + Tier-2.
    switchpost::config::load_dotenv_for_tests();
    let config = switchpost::config::Config::from_env().expect("config");
    let registry =
        switchpost::social::registry::ProviderRegistry::new(&config, None, None);
    let count = registry.list().len();
    assert!(count > 0, "the registry registered no providers");

    let tier_1 = switchpost::social::tier::TIER_1.len();
    let tier_2 = switchpost::social::tier::TIER_2.len();
    assert_eq!(
        count,
        tier_1 + tier_2,
        "the registry registers Tier-1 + Tier-2 by default; a mismatch means \
         the default set and the tier constants have drifted apart"
    );

    // Every doc that states the count must state the TRUE one, and must
    // not carry the stale figure that was there until 2026-11-04.
    let agents = std::fs::read_to_string(repo_path("AGENTS.md")).unwrap();
    assert!(
        agents.contains(&format!("**{count} providers registered by default**")),
        "AGENTS.md must state the true registered-provider count \
         ({count} providers registered by default)"
    );
    assert!(
        !agents.contains("26 providers registered by default"),
        "AGENTS.md still states the stale 26-provider count"
    );

    let readme = std::fs::read_to_string(repo_path("README.md")).unwrap();
    assert!(
        readme.contains(&format!("**{count} social platforms**")),
        "README.md must state the true platform count ({count} social platforms)"
    );
    for stale in ["26 social platforms", "26 platforms", "14 Tier-2", "14 T2"] {
        assert!(
            !readme.contains(stale),
            "README.md still states the stale provider count ({stale})"
        );
    }

    // The agent skill is what an AI agent actually reads, so a stale count
    // there is worse than one in a README.
    let skill = std::fs::read_to_string(repo_path("skills/switchpost-agent/SKILL.md")).unwrap();
    assert!(
        skill.contains(&format!("{count} social platforms")),
        "the agent skill must state the true platform count \
         ({count} social platforms)"
    );
    assert!(
        !skill.contains("26 social platforms"),
        "the agent skill still states the stale 26-platform count"
    );
}

#[test]
fn d1_documented_cli_command_count_matches_clap() {
    use clap::CommandFactory;
    let commands = switchpost::cli::Cli::command()
        .get_subcommands()
        .filter(|c| c.get_name() != "help")
        .count();
    assert!(commands > 0, "no CLI commands parsed");
    // The repository layout (which states this count) lives in docs/platforms-and-cli.md.
    let layout = std::fs::read_to_string(repo_path("docs/platforms-and-cli.md")).unwrap();
    assert!(
        layout.contains(&format!("clap, {commands} commands")),
        "docs/platforms-and-cli.md must state the true CLI command count ({commands} commands)"
    );
}

#[test]
fn d2_tool_schema_shape() {
    let tools = parse_tools();
    let mut names: Vec<&str> = Vec::with_capacity(tools.len());
    for (name, desc) in &tools {
        assert!(!desc.is_empty(), "{name}: empty description");
        // Schema-size discipline (17.9k chars total today; median 521B):
        // one tight line per tool keeps tools/list affordable.
        assert!(
            desc.len() <= 240,
            "{name}: description {} chars > 240 — trim it",
            desc.len()
        );
        // Provider-prefixed names: the first `_` segment is the area
        // (posts_, x_, reddit_, tu_, …). tools_search and tools_enable
        // are the documented meta exceptions.
        assert!(name.contains('_'), "{name}: no area prefix");
        assert!(!name.starts_with('_'), "{name}: leading underscore");
        names.push(name);
    }
    let mut sorted = names.clone();
    sorted.sort_unstable();
    let before = sorted.len();
    sorted.dedup();
    assert_eq!(before, sorted.len(), "duplicate tool names registered");
}

/// D3: the mcp-catalogue's per-platform tool counts vs the registered counts.
///
/// v0.5.0 of the agent skill RETIRED `references/providers.md` (it carried the
/// wrong providers -> internal_id claim) and split it into
/// `platform-matrices.md` + `mcp-catalogue.md`. This check kept pointing at the
/// retired file, so it panicked "no such file" on every run - which nobody saw,
/// because the whole suite could not BUILD until the .sqlx cache was repaired on
/// 2026-11-01. A test that cannot run is the same blindness as a gate that
/// measures nothing, one layer down.
///
/// D1 asserts the TOTAL count, which passed for weeks while eight of the
/// fourteen per-platform counts underneath it were stale. A stale
/// per-platform number is worse than no number: an agent reads it and
/// concludes a whole capability area is missing, or does not discover the
/// one it needs. This walks the `### Platform (N tools)` headings and
/// compares N against the tools actually registered under that prefix,
/// including the tools the section's body lists, so a body that quietly
/// shrinks is caught too.
///
/// The catalogue's heading shape is `## X (x_, 27 tools)` rather than
/// `### Platform (N tools)`, and it groups platforms in five different ways
/// (see the table inside the test body), so the parser below is written for
/// those shapes rather than for one.
#[test]
fn d3_documented_per_platform_counts_match_registry() {
    let providers = std::fs::read_to_string(repo_path(
        "skills/switchpost-agent/references/mcp-catalogue.md",
    ))
    .expect("read mcp-catalogue.md");

    // Real count per tool-name prefix, from the registry itself.
    let mut real: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (name, _) in parse_tools() {
        let prefix = name.split('_').next().unwrap_or(&name).to_string();
        *real.entry(prefix).or_insert(0) += 1;
    }

    // The catalogue states each platform's tool count beside its prefix, and
    // groups platforms in five shapes:
    //
    //   (x_, 27 tools)                          one prefix
    //   (ig_ + ias_, 21+7 tools)                 two prefixes, numbers after both
    //   (li_ personal 15 + lip_ page 11 tools)   two prefixes, each labelled
    //   (bs_, 7 tools) / Mastodon (ms_, 6 tools) two platforms, two paren groups
    //   (wp_, 4) / Medium (md_, 3) / ...         four tag-only blog prefixes
    //
    // Reading the first `(N tools)` on the line is wrong in three of the five:
    // it credits Mastodon with Bluesky's 7, reads Instagram's 21 as its total,
    // and misses every blog prefix entirely. Summing the whole line is wrong
    // too, because `th_` and `tt_` share a line and Threads would absorb
    // TikTok's 4. So the declared figure is parsed as the group's numbers, in
    // order, one per prefix named in it.
    //
    // Each heading is listed once per platform it covers - the Bluesky line
    // appears as both `bs_ -> {bs_, ms_}` and `ms_ -> {bs_, ms_}` - so every
    // platform's count is checked on its own, and a heading that adds a second
    // platform without a matching entry fails the coverage check below.
    let sections: &[(&str, &[&str])] = &[
        ("x_", &["x"]),
        ("reddit_", &["reddit"]),
        ("goog_", &["goog"]),
        ("gh_", &["gh"]),
        ("fb_", &["fb"]),
        ("ig_", &["ig", "ias"]),
        ("li_", &["li", "lip"]),
        ("tb_", &["tb", "tu"]),
        ("di_", &["di"]),
        ("wa_", &["wa", "wc"]),
        ("bs_", &["bs", "ms"]),
        ("ms_", &["bs", "ms"]),
        ("yt_", &["yt"]),
        ("th_", &["th", "tt"]),
        ("tt_", &["th", "tt"]),
        ("pi_", &["pi"]),
        ("wp_", &["wp", "md", "dv", "hn"]),
        ("md_", &["wp", "md", "dv", "hn"]),
        ("dv_", &["wp", "md", "dv", "hn"]),
        ("hn_", &["wp", "md", "dv", "hn"]),
        ("sk_", &["sk", "sl"]),
        ("sl_", &["sk", "sl"]),
    ];

    let lines: Vec<&str> = providers.lines().collect();

    // Every per-platform heading in the catalogue must be COVERED by a section
    // above, and covered EXACTLY: the set of prefixes named on the heading must
    // equal the set the section checks. A heading that states counts nobody
    // checks is the whole failure mode this test exists to close - it is how
    // Hashnode, Medium and Dev.to sat unmeasured while WordPress (same heading,
    // four numbers) was checked, and how a second platform added later to a
    // checked heading would sit unmeasured while the heading still looked
    // checked. So a heading naming `(pi_, ... / fc_, ...)` with a section that
    // only knows `pi_` is a failure, not a pass.
    let mut covered_lines: Vec<usize> = Vec::new();
    for (primary, prefixes) in sections {
        let idx = lines
            .iter()
            .position(|l| l.starts_with("## ") && l.contains(&format!("({primary}")))
            .unwrap_or_else(|| panic!("mcp-catalogue.md: no section for prefix `{primary}` ("));
        let named: Vec<String> = lines[idx]
            .split(|c: char| c == '(' || c == ')' || c == ' ' || c == ':' || c == '/' || c == ',')
            .filter(|seg| {
                seg.ends_with('_') && seg.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            })
            .map(|s| s.to_string())
            .collect();
        let expected: Vec<String> = prefixes.iter().map(|p| format!("{p}_")).collect();
        let mut named_sorted = named.clone();
        let mut expected_sorted = expected.clone();
        named_sorted.sort();
        expected_sorted.sort();
        assert_eq!(
            named_sorted, expected_sorted,
            "mcp-catalogue.md: heading `{}` names prefixes {named:?} but the section checks {expected:?}",
            lines[idx]
        );
        covered_lines.push(idx);
    }
    // ... and every heading that STATES per-platform counts must be among those
    // covered headings, so a new platform heading cannot appear unmeasured.
    for (i, line) in lines.iter().enumerate() {
        if !line.starts_with("## ") {
            continue;
        }
        let named: Vec<String> = line
            .split(|c: char| c == '(' || c == ')' || c == ' ' || c == ':' || c == '/' || c == ',')
            .filter(|seg| {
                seg.ends_with('_') && seg.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            })
            .map(|s| s.to_string())
            .collect();
        if named.is_empty() {
            continue;
        }
        assert!(
            covered_lines.contains(&i),
            "mcp-catalogue.md: heading `{line}` states per-platform counts but no section checks it"
        );
    }

    let mut checked = 0;
    for (primary, prefixes) in sections {
        let idx = lines
            .iter()
            .position(|l| l.starts_with("## ") && l.contains(&format!("({primary}")))
            .unwrap_or_else(|| panic!("mcp-catalogue.md: no section for prefix `{primary}` ("));
        // The group is the whole parenthesised region of the heading, held
        // together by BOTH paren pairs when a line carries two platforms
        // (`## Bluesky (bs_, 7 tools) / Mastodon (ms_, 6 tools)`). Taking only
        // the primary's own pair would check Bluesky and silently skip
        // Mastodon, so the group runs from the FIRST open paren to the LAST
        // close paren on the line. It must name every prefix the section
        // covers, and state exactly one number per prefix - the pairing rule is
        // ordered, so it does not need to know which number belonged to which
        // label:
        //   (ig_ + ias_, 21+7 tools)   [21, 7]   both numbers after both labels
        //   (wa_ 9 + wc_ 4 tools)      [9, 4]    each number after its label
        //   (bs_, 7 tools) / (ms_, 6)  [7, 6]    two pairs, order preserved
        // A heading that names a prefix but forgets its number, or states two
        // numbers for one prefix, is REJECTED here rather than silently summing
        // to a figure that happens to reconcile.
        let group_start = lines[idx].find('(').unwrap_or_else(|| {
            panic!(
                "mcp-catalogue.md: heading `{}` has no count parens",
                lines[idx]
            )
        });
        let group_end = lines[idx]
            .rfind(')')
            .map(|p| p + 1)
            .unwrap_or(lines[idx].len());
        let group = &lines[idx][group_start..group_end];
        for prefix in *prefixes {
            assert!(
                group.contains(&format!("{prefix}_")),
                "mcp-catalogue.md: heading `{}` never names `{prefix}_`, so its count is unchecked",
                lines[idx]
            );
        }
        let numbers: Vec<usize> = group
            .split(|c: char| !c.is_ascii_digit())
            .filter(|w| !w.is_empty())
            .map(|w| w.parse::<usize>().unwrap_or(0))
            .collect();
        assert_eq!(
            numbers.len(),
            prefixes.len(),
            "mcp-catalogue.md: heading `{}` names {} prefixes but states {} numbers",
            lines[idx],
            prefixes.len(),
            numbers.len()
        );
        let declared: usize = numbers.iter().sum();

        // The section body, up to the next heading or `---`. The appended
        // "Not listed above" continuation line is INSIDE the body on purpose:
        // some sections are a curated subset plus an explicit continuation,
        // and the whole thing must reconcile. What the check must catch is a
        // section that silently drops tools with no continuation saying so —
        // so the continuation line is counted, and then the drop is required
        // to be declared rather than inferred.
        let mut body = String::new();
        let mut continuation = false;
        for line in &lines[idx + 1..] {
            if line.starts_with("### ") || line.starts_with("## ") || line.trim() == "---" {
                break;
            }
            if line.starts_with("**Not listed above") {
                continuation = true;
            }
            body.push_str(line);
            body.push('\n');
        }

        let expected: usize = prefixes.iter().map(|p| *real.get(*p).unwrap_or(&0)).sum();
        let mut listed = 0usize;
        for prefix in *prefixes {
            for (name, _) in parse_tools() {
                if name.starts_with(&format!("{prefix}_")) && body.contains(&name) {
                    listed += 1;
                }
            }
        }

        assert_eq!(
            declared, expected,
            "mcp-catalogue.md: `{primary}` claims {declared} tools but the registry has {expected}"
        );
        // A section may be a curated subset, but the omission must be DECLARED by a
        // continuation line that names the omitted tools. Requiring merely that
        // some continuation line exists is not enough: a stale line left over from
        // an earlier edit satisfies it while the drop is invisible. So the
        // continuation's own tail count is reconciled against what the prose lists.
        if listed < expected {
            let tail = lines[idx + 1..]
                .iter()
                .take_while(|l| {
                    !l.starts_with("### ") && !l.starts_with("## ") && l.trim() != "---"
                })
                .find(|l| l.starts_with("**Not listed above"))
                .and_then(|l| {
                    let after = l.split("Not listed above").nth(1)?;
                    let n = after.trim().trim_start_matches('(').split(' ').next()?;
                    n.parse::<usize>().ok()
                })
                .unwrap_or(0);
            assert!(
                continuation,
                "mcp-catalogue.md: `{primary}` names {listed} of {expected} registered tools and has no \
                 `**Not listed above` continuation — add the missing tools or say which are omitted"
            );
            assert_eq!(
                listed + tail,
                expected,
                "mcp-catalogue.md: `{primary}` lists {listed} tools and its continuation claims {tail} more, \
                 totalling {} — the registry has {expected}. The continuation must name every tool \
                 the prose omits, and its count must be true",
                listed + tail
            );
        }
        checked += 1;
    }
    assert_eq!(checked, sections.len(), "every section must be checked");
}
