use anyhow::Result;
use tracing::info;

use crate::llm::client::{ChatMessage, LlmClient};

/// Rewrite known ambiguous tech terms so search queries target the right thing.
/// Only substitutes when the term appears as a standalone word and isn't already qualified.
fn disambiguate_topic(topic: &str) -> String {
    // (term, replacement) — order matters: more specific first to avoid double-expanding.
    // The Rust `regex` crate does not support lookbehind; use \b word boundaries instead.
    const TECH_TERMS: &[(&str, &str)] = &[
        ("Rust lang",                  "Rust programming language"),
        ("Rust",                       "Rust programming language"),
        ("Golang",                     "Go programming language"),
        ("Swift",                      "Swift programming language"),
        ("Haskell",                    "Haskell programming language"),
        ("Elixir",                     "Elixir programming language"),
        ("Crystal",                    "Crystal programming language"),
        ("Nim",                        "Nim programming language"),
    ];

    let mut result = topic.to_string();
    // TODO: cache compiled regexes with std::sync::LazyLock to avoid re-compiling on every call
    for (term, replacement) in TECH_TERMS {
        // Skip if already qualified
        if result.contains(replacement) {
            continue;
        }
        let pattern = format!(r"\b{term}\b");
        if let Ok(re) = regex::Regex::new(&pattern) {
            result = re.replace_all(&result, *replacement).into_owned();
        }
    }
    result
}


/// Expands well-known ticker symbols to their full name for better search quality.
/// E.g. "BTC" → "Bitcoin", "NVDA" → "Nvidia".
fn expand_ticker(topic: &str) -> String {
    const TICKERS: &[(&str, &str)] = &[
        ("BTC",   "Bitcoin"),
        ("ETH",   "Ethereum"),
        ("SOL",   "Solana"),
        ("BNB",   "Binance Coin"),
        ("XRP",   "Ripple XRP"),
        ("ADA",   "Cardano"),
        ("DOGE",  "Dogecoin"),
        ("AVAX",  "Avalanche"),
        ("NVDA",  "Nvidia"),
        ("AAPL",  "Apple"),
        ("MSFT",  "Microsoft"),
        ("GOOGL", "Google Alphabet"),
        ("META",  "Meta Platforms"),
        ("AMZN",  "Amazon"),
        ("TSLA",  "Tesla"),
        ("AMD",   "AMD Advanced Micro Devices"),
        ("INTC",  "Intel"),
        ("SPY",   "S&P 500 ETF"),
        ("QQQ",   "Nasdaq 100 ETF"),
    ];
    let mut result = topic.to_string();
    // TODO: cache compiled regexes with std::sync::LazyLock to avoid re-compiling on every call
    for (ticker, name) in TICKERS {
        if result.contains(name) {
            continue;
        }
        // Match ticker as whole word, with optional leading $
        let pattern = format!(r"\$?(?i)\b{ticker}\b");
        if let Ok(re) = regex::Regex::new(&pattern) {
            result = re.replace_all(&result, *name).into_owned();
        }
    }
    result
}
    // TODO: cache compiled regexes with std::sync::LazyLock to avoid re-compiling on every call


/// Ask the LLM to decompose a research query into focused sub-questions.
/// Returns a list of search queries to run in parallel.
pub async fn generate_queries(
    llm: &LlmClient,
    topic: &str,
    max_queries: usize,
    domains: &[String],
    target: &crate::researcher::pipeline::ResearchTarget,
) -> Result<Vec<String>> {
    let topic = &disambiguate_topic(topic);
    info!(%topic, "planning research queries");

    use crate::researcher::pipeline::{ResearchTarget, PersonMethod};

    let system_prompt = match target {
        ResearchTarget::Person { method } => {
            let focus = match method {
                PersonMethod::Company  => "professional background, career, expertise, public work, and thought leadership",
                PersonMethod::Personal => "personal interests, hobbies, lifestyle, and online presence",
                PersonMethod::Both     => "both professional background and personal interests/hobbies",
            };
            format!(
                "You are a research planning assistant specializing in people research. \
                 Generate focused search queries to build a profile of a person — covering their {focus}. \
                 Use their name in every query. Be specific and targeted.\n\
                 IMPORTANT: Write queries as plain search terms only — no dashes, no hyphens between concepts, \
                 no possessives with apostrophes, no special operators. \
                 Good: 'Andrej Karpathy OpenAI career history' \
                 Bad: 'Andrej Karpathy - OpenAI role' or 'Karpathy\\'s projects'"
            )
        }
        ResearchTarget::Company => {
            "You are a research planning assistant specializing in company research. \
             Generate focused search queries covering: what the company does, its size and funding stage, \
             recent news and launches, culture and values, and strategic priorities. \
             Use the company name in every query. \
             Write queries as plain search terms — no dashes between concepts, no special operators.".to_string()
        }
        ResearchTarget::Topic => {
            "You are a research planning assistant. Your job is to decompose a research \
             topic into specific, focused search queries that together will provide \
             comprehensive coverage of the topic. Each query should target a different \
             angle or subtopic. Be specific and use natural language search terms. \
             IMPORTANT: If the topic contains ambiguous terms that could refer to multiple \
             things (e.g. a programming language name that is also a common word or product), \
             always include disambiguating context in every query — for example, prefer \
             'Rust programming language async' over just 'Rust async'.".to_string()
        }
        ResearchTarget::Market { asset_class } => {
            use crate::researcher::pipeline::AssetClass;
            let focus = match asset_class {
                AssetClass::Stock => "recent earnings and analyst guidance, analyst upgrades and downgrades, \
                    sector tailwinds and headwinds, macro events affecting the stock, \
                    institutional positioning and short interest",
                AssetClass::Crypto => "on-chain metrics such as TVL active addresses and exchange flows, \
                    protocol upgrades and ecosystem news, regulatory developments, \
                    whale activity and exchange reserves, market sentiment and fear greed index",
                AssetClass::Macro => "central bank signals and interest rate outlook, \
                    inflation employment and GDP data, geopolitical risk events, \
                    credit market stress indicators, sector rotation and risk sentiment",
            };
            format!(
                "You are a research planning assistant specializing in financial market research. \
                 Generate focused search queries covering: {focus}. \
                 Use the asset name or topic in every query. Be specific and targeted. \
                 Write queries as plain search terms only — no dashes, no hyphens, no special operators."
            )
        }
    };

    let domain_instruction = if !domains.is_empty() {
        let is_profile_target = matches!(
            target,
            ResearchTarget::Person { .. } | ResearchTarget::Company | ResearchTarget::Market { .. }
        );
        if is_profile_target {
            // Soft hints: use preferred domains for some queries, open-web for others
            let preferred = domains.join(", ");
            format!(
                "\n\nFor at least half your queries, include the subject name (person, company, or asset) \
                 relevant context as a plain open-web search (no site filter). \
                 For the remaining queries you MAY use site filters from these preferred sources \
                 if they are likely to have the information: {preferred}"
            )
        } else {
            // Hard filter for explicit domain overrides (e.g. shopping profiles)
            let domain_list = domains
                .iter()
                .map(|d| format!("site:{d}"))
                .collect::<Vec<_>>()
                .join(" OR ");
            let allowed = domains.join(", ");
            format!(
                "\n\nIMPORTANT: Restrict ALL queries to these domains only. Each query MUST include a site filter.\n\
                 Example format: your search terms {domain_list}\n\
                 Allowed domains: {allowed}"
            )
        }
    } else {
        String::new()
    };

    let display_topic = if matches!(target, ResearchTarget::Market { .. }) {
        expand_ticker(topic)
    } else {
        topic.to_string()
    };

    let messages = vec![
        ChatMessage::system(system_prompt),
        ChatMessage::user(format!(
            "Research topic: {display_topic}\n\n\
             Generate exactly {max_queries} distinct search queries to research this topic \
             comprehensively. Each query should be on its own line, with no numbering, \
             bullets, quotes, or extra formatting — just the raw query text.{domain_instruction}",
        )),
    ];

    let response = llm.complete(messages).await?;

    let queries: Vec<String> = response
        .lines()
        .map(|l| l.trim().trim_start_matches(['-', '*', '•', '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '.', ')', '"']))
        .map(|l| l.trim_end_matches(['"', '.', ',']))
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.len() > 5)
        .take(max_queries)
        .map(|q| disambiguate_topic(q))
        .collect();

    info!(count = queries.len(), ?queries, "generated search queries");
    Ok(queries)
}
