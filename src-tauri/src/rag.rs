use std::{
    collections::HashSet,
    fs,
    path::Path,
    time::Duration,
};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::{
    config::AiProviderConfig,
    vault::{is_markdown, list_vault_items, safe_join},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagChunk {
    pub note_path: String,
    pub note_title: String,
    pub section_title: String,
    pub line_number: usize,
    pub content: String,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub answer: String,
    pub sources: Vec<RagChunk>,
    pub web_sources: Vec<crate::assistant::WebSource>,
    pub drafts: Vec<crate::assistant::NoteDraft>,
    pub edits: Vec<crate::assistant::NoteEdit>,
    pub warnings: Vec<String>,
    pub vault_id: String,
}

#[derive(Debug, Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatChoice {
    message: OpenAiChatMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChatChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelItem {
    id: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModelItem>,
}

/// Search vault notes for relevant chunks using keyword, heading and phrase matching
pub fn index_and_search_vault(
    vault_path: &Path,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<RagChunk>> {
    let clean_query = query.trim().to_lowercase();
    if clean_query.is_empty() {
        return Ok(Vec::new());
    }

    let query_terms: Vec<String> = clean_query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_string())
        .collect();

    let query_set: HashSet<&str> = query_terms.iter().map(|s| s.as_str()).collect();

    let items = list_vault_items(vault_path)?;
    let mut all_chunks = Vec::new();

    for item in items {
        if item.is_dir || !is_markdown(Path::new(&item.path)) {
            continue;
        }

        let full_path = safe_join(vault_path, &item.path)?;
        let content = match fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let note_chunks = chunk_markdown(&item.path, &item.title, &content);
        for mut chunk in note_chunks {
            let score = score_chunk(&chunk, &clean_query, &query_set);
            if score > 0.0 {
                chunk.score = score;
                all_chunks.push(chunk);
            }
        }
    }

    all_chunks.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    all_chunks.truncate(limit);
    Ok(all_chunks)
}

fn chunk_markdown(note_path: &str, note_title: &str, content: &str) -> Vec<RagChunk> {
    let mut chunks = Vec::new();
    let mut current_section = "Introduction".to_string();
    let mut current_lines = Vec::new();
    let mut chunk_start_line = 1;

    for (line_idx, line) in content.lines().enumerate() {
        let line_num = line_idx + 1;
        let trimmed = line.trim();

        if trimmed.starts_with('#') {
            // Heading detected - flush current chunk if not empty
            if !current_lines.is_empty() {
                chunks.push(RagChunk {
                    note_path: note_path.to_string(),
                    note_title: note_title.to_string(),
                    section_title: current_section.clone(),
                    line_number: chunk_start_line,
                    content: current_lines.join("\n"),
                    score: 0.0,
                });
                current_lines.clear();
            }

            current_section = trimmed
                .trim_start_matches('#')
                .trim()
                .to_string();
            chunk_start_line = line_num;
            current_lines.push(line.to_string());
        } else if trimmed.is_empty() && current_lines.len() >= 12 {
            // Paragraph break when chunk is already substantial
            chunks.push(RagChunk {
                note_path: note_path.to_string(),
                note_title: note_title.to_string(),
                section_title: current_section.clone(),
                line_number: chunk_start_line,
                content: current_lines.join("\n"),
                score: 0.0,
            });
            current_lines.clear();
            chunk_start_line = line_num + 1;
        } else {
            if current_lines.is_empty() {
                chunk_start_line = line_num;
            }
            current_lines.push(line.to_string());
        }
    }

    if !current_lines.is_empty() {
        chunks.push(RagChunk {
            note_path: note_path.to_string(),
            note_title: note_title.to_string(),
            section_title: current_section,
            line_number: chunk_start_line,
            content: current_lines.join("\n"),
            score: 0.0,
        });
    }

    chunks
}

fn score_chunk(chunk: &RagChunk, full_query: &str, query_terms: &HashSet<&str>) -> f32 {
    let lower_content = chunk.content.to_lowercase();
    let lower_title = chunk.note_title.to_lowercase();
    let lower_section = chunk.section_title.to_lowercase();
    let lower_path = chunk.note_path.to_lowercase();

    let mut score = 0.0;

    // Exact phrase match
    if lower_content.contains(full_query) {
        score += 25.0;
    }
    if lower_title.contains(full_query) || lower_path.contains(full_query) {
        score += 35.0;
    }
    if lower_section.contains(full_query) {
        score += 20.0;
    }

    // Individual keyword hits
    for term in query_terms {
        if lower_title.contains(term) {
            score += 10.0;
        }
        if lower_section.contains(term) {
            score += 6.0;
        }
        if lower_path.contains(term) {
            score += 8.0;
        }

        let occurrences = lower_content.matches(term).count();
        if occurrences > 0 {
            score += (occurrences.min(5) as f32) * 2.0;
        }
    }

    score
}

/// Fetch list of models from any OpenAI-compatible provider
pub async fn fetch_models(
    base_url: &str,
    api_key: &str,
) -> anyhow::Result<Vec<String>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()?;

    let mut url = base_url.trim().trim_end_matches('/').to_string();
    if !url.ends_with("/models") {
        url.push_str("/models");
    }

    let mut req = client.get(&url);
    if !api_key.trim().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key.trim()));
    }

    let resp = req.send().await.context("falha ao conectar ao provedor de IA")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("erro do provedor (status {status}): {body}");
    }

    let body: OpenAiModelsResponse = resp
        .json()
        .await
        .context("errors.invalidModelsResponse")?;

    let mut models: Vec<String> = body.data.into_iter().map(|m| m.id).collect();
    models.sort();
    Ok(models)
}

/// Execute chat completion call to OpenAI-compatible provider
pub async fn generate_chat_completion(
    provider: &AiProviderConfig,
    messages: &[ChatMessage],
    temperature: f32,
) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()?;

    let mut url = provider.base_url.trim().trim_end_matches('/').to_string();
    if !url.ends_with("/chat/completions") {
        url.push_str("/chat/completions");
    }

    let model = if !provider.selected_model.trim().is_empty() {
        provider.selected_model.trim()
    } else {
        "gpt-4o-mini"
    };

    let payload = OpenAiChatRequest {
        model,
        messages,
        temperature,
    };

    let mut req = client.post(&url).json(&payload);
    if !provider.api_key.trim().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", provider.api_key.trim()));
    }

    let resp = req.send().await.map_err(|error| {
        let key = if error.is_timeout() {
            "ai.providerTimeout"
        } else if error.is_connect() {
            "ai.providerConnectionFailed"
        } else {
            "ai.providerRequestFailed"
        };
        anyhow::anyhow!(key)
    })?;
    if !resp.status().is_success() {
        let status = resp.status();
        let key = match status.as_u16() {
            401 | 403 => "ai.providerAuthFailed",
            404 => "ai.providerEndpointNotFound",
            429 => "ai.providerRateLimited",
            500..=599 => "ai.providerUnavailable",
            _ => "ai.providerRejected",
        };
        bail!(key);
    }

    let result: OpenAiChatResponse = resp.json().await.map_err(|error: reqwest::Error| {
        anyhow::anyhow!(if error.is_timeout() { "ai.providerTimeout" } else { "errors.invalidAiResponse" })
    })?;

    let choice = result
        .choices
        .into_iter()
        .next()
        .context("errors.aiEmptyResponse")?;

    let answer = choice
        .message
        .content
        .unwrap_or_else(|| "errors.aiEmptyContent".to_string());

    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunking_and_scoring() {
        let text = "# Introdução\nEste é o LowNotes com P2P.\n\n## Arquitetura\nA sincronização usa Iroh e CRDTs.\n";
        let chunks = chunk_markdown("test.md", "LowNotes Test", text);
        assert!(!chunks.is_empty());

        let mut query_set = HashSet::new();
        query_set.insert("iroh");
        let score = score_chunk(&chunks[1], "iroh", &query_set);
        assert!(score > 0.0);
    }
}
