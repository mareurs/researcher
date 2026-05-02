use std::path::PathBuf;
use anyhow::Result;
use chrono::Local;
use crate::llm::client::LlmClient;
use crate::researcher::pipeline::ResearchResult;

#[derive(Debug, serde::Serialize)]
pub struct ProgressiveEnvelope {
    pub summary: String,
    pub toc: Vec<String>,
    pub path: String,
    pub word_count: usize,
    pub hint: &'static str,
}

pub enum MaybeDisclosed {
    Inline(String),
    Progressive(String),
}

impl MaybeDisclosed {
    pub fn into_string(self) -> String {
        match self {
            MaybeDisclosed::Inline(s) | MaybeDisclosed::Progressive(s) => s,
        }
    }
}

pub fn extract_toc(report: &str) -> Vec<String> {
    report
        .lines()
        .filter(|line| line.starts_with("## "))
        .map(str::to_string)
        .collect()
}

pub fn write_to_disk(topic: &str, report: &str) -> Result<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let data_dir = PathBuf::from(home).join(".local/share/researcher");
    std::fs::create_dir_all(&data_dir)?;

    let slug: String = topic
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug: String = slug.chars().take(40).collect::<String>();
    let slug = slug.trim_matches('-');

    let ts = Local::now().format("%Y-%m-%dT%H-%M-%S");
    let filename = format!("{ts}-{slug}.md");
    let path = data_dir.join(filename);
    std::fs::write(&path, report)?;
    Ok(path)
}

pub async fn generate_summary(llm: &LlmClient, report: &str, style: &str) -> Result<String> {
    todo!()
}

pub async fn maybe_disclose(
    llm: &LlmClient,
    topic: &str,
    result: &ResearchResult,
    summary_style: &str,
    threshold: usize,
) -> Result<MaybeDisclosed> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_toc_returns_h2_headings() {
        let report = "# Title\n\nSome text.\n\n## Executive Summary\n\nMore.\n\n## Key Findings\n\n- foo\n\n### Subsection\n\n## Sources\n";
        let toc = extract_toc(report);
        assert_eq!(toc, vec![
            "## Executive Summary".to_string(),
            "## Key Findings".to_string(),
            "## Sources".to_string(),
        ]);
    }

    #[test]
    fn extract_toc_empty_report() {
        assert!(extract_toc("").is_empty());
    }

    #[test]
    fn extract_toc_no_h2() {
        let report = "# Title\n\n### Only h3\n\ntext";
        assert!(extract_toc(report).is_empty());
    }

    #[test]
    fn write_to_disk_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());
        let path = write_to_disk("test topic", "# Report\n\nContent here.").unwrap();
        assert!(path.exists());
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "# Report\n\nContent here.");
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with("-test-topic.md"), "got: {name}");
    }

    #[test]
    fn write_to_disk_slug_strips_specials() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());
        let path = write_to_disk("Rust async/await: deep dive!", "content").unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '.'));
    }
}
