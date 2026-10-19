use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{bail, Context};
use reqwest::{Client, Url};
use serde_json::{json, Value};

use crate::{
    assistant::WebSource,
    config::{WebSearchSettings, WebSearchSourceConfig},
};

static NEXT_SEARCH: AtomicUsize = AtomicUsize::new(0);
const ORDER: [&str; 7] = [
    "firecrawl",
    "keenable",
    "exa",
    "duckduckgo",
    "searxng",
    "brave",
    "parallel",
];

pub async fn search_web(
    query: &str,
    settings: &WebSearchSettings,
    language: &str,
) -> anyhow::Result<Vec<WebSource>> {
    let query = query
        .split_whitespace()
        .take(75)
        .collect::<Vec<_>>()
        .join(" ");
    let query: String = query.chars().take(400).collect();
    if query.is_empty() {
        bail!("ai.emptyPrompt");
    }
    let enabled: Vec<_> = ORDER
        .into_iter()
        .filter_map(|id| {
            settings.source(id).filter(|source| {
                source.enabled
                    && (!matches!(id, "brave" | "parallel") || !source.api_key.trim().is_empty())
            })
        })
        .collect();
    if enabled.is_empty() {
        bail!("ai.webNoProviders");
    }
    static SEARCH_CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();
    let client = crate::http::cached(&SEARCH_CLIENT, |builder| builder.timeout(Duration::from_secs(12)));
    let start = NEXT_SEARCH.fetch_add(1, Ordering::Relaxed) % enabled.len();
    for offset in 0..enabled.len() {
        let source = enabled[(start + offset) % enabled.len()];
        match search_source(&client, &query, language, source, settings).await {
            Ok(results) => {
                let results = clean_results(results);
                if !results.is_empty() {
                    return Ok(results);
                }
                eprintln!("web search {} returned no usable results", source.id);
            }
            Err(_) => eprintln!("web search {} failed", source.id),
        }
    }
    bail!("ai.webSearchFailed")
}

fn clean_results(results: Vec<WebSource>) -> Vec<WebSource> {
    results
        .into_iter()
        .filter_map(|mut result| {
            let url = Url::parse(&result.url).ok()?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return None;
            }
            result.title = result.title.chars().take(300).collect();
            result.description = result.description.chars().take(1800).collect();
            if result.title.trim().is_empty() {
                return None;
            }
            Some(result)
        })
        .take(5)
        .collect()
}

async fn search_source(
    client: &Client,
    query: &str,
    language: &str,
    source: &WebSearchSourceConfig,
    settings: &WebSearchSettings,
) -> anyhow::Result<Vec<WebSource>> {
    let key = source.api_key.trim();
    let value: Value = match source.id.as_str() {
        "firecrawl" => {
            let mut request = client
                .post("https://api.firecrawl.dev/v2/search")
                .json(&json!({"query":query,"limit":5}));
            if !key.is_empty() {
                request = request.bearer_auth(key);
            }
            checked_json(request).await?
        }
        "keenable" => {
            let endpoint = if key.is_empty() {
                "https://api.keenable.ai/v1/search/public"
            } else {
                "https://api.keenable.ai/v1/search"
            };
            let mut request = client
                .post(endpoint)
                .header("X-Keenable-Title", "LowNotes")
                .json(&json!({"query":query,"mode":"pro","snippet_max_length":1200}));
            if !key.is_empty() {
                request = request.header("X-API-Key", key);
            }
            checked_json(request).await?
        }
        "exa" => {
            let mut request = client.post("https://mcp.exa.ai/mcp")
                .header("Accept", "application/json, text/event-stream")
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"web_search_exa","arguments":{"query":query,"numResults":5}}}));
            if !key.is_empty() {
                request = request.header("x-api-key", key);
            }
            let response = request.send().await?.error_for_status()?;
            let body = response.text().await?;
            let payload = body
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .unwrap_or(body.as_str());
            serde_json::from_str(payload).context("invalid Exa MCP response")?
        }
        "duckduckgo" => {
            let mut url = Url::parse("https://html.duckduckgo.com/html/")?;
            url.query_pairs_mut().append_pair("q", query);
            let body = client
                .get(url)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            return Ok(parse_duckduckgo(&body));
        }
        "searxng" => {
            let mut url = Url::parse(settings.searxng_url.trim())?;
            if url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                bail!("ai.webInvalidSearxngUrl");
            }
            url.set_path("/search");
            url.query_pairs_mut()
                .clear()
                .append_pair("q", query)
                .append_pair("format", "json")
                .append_pair("language", language);
            checked_json(client.get(url).header("Accept", "application/json")).await?
        }
        "brave" => {
            let mut url = Url::parse("https://api.search.brave.com/res/v1/web/search")?;
            let (country, search_lang) = match language {
                "pt-BR" => ("BR", "pt-br"),
                "es-ES" => ("ES", "es"),
                _ => ("US", "en"),
            };
            url.query_pairs_mut().extend_pairs([
                ("q", query),
                ("count", "5"),
                ("country", country),
                ("search_lang", search_lang),
                ("text_decorations", "false"),
            ]);
            checked_json(client.get(url).header("X-Subscription-Token", key)).await?
        }
        "parallel" => {
            checked_json(
                client
                    .post("https://api.parallel.ai/v1/search")
                    .header("x-api-key", key)
                    .json(&json!({"objective":query,"search_queries":[query],"mode":"fast"})),
            )
            .await?
        }
        _ => bail!("unknown search source"),
    };
    Ok(parse_json_results(&source.id, &value))
}

async fn checked_json(request: reqwest::RequestBuilder) -> anyhow::Result<Value> {
    let response = request.send().await?.error_for_status()?;
    Ok(response.json().await?)
}

fn parse_json_results(source: &str, value: &Value) -> Vec<WebSource> {
    if source == "exa" {
        let text = value
            .pointer("/result/content/0/text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return text
            .split("\n---\n")
            .filter_map(|block| {
                let title = block
                    .lines()
                    .find_map(|line| line.strip_prefix("Title: "))?;
                let url = block.lines().find_map(|line| line.strip_prefix("URL: "))?;
                let description = block
                    .split_once("Highlights:\n")
                    .map(|(_, body)| body.trim())
                    .unwrap_or_default();
                Some(WebSource {
                    title: title.into(),
                    url: url.into(),
                    description: description.into(),
                })
            })
            .collect();
    }
    let pointer = match source {
        "firecrawl" => "/data/web",
        "brave" => "/web/results",
        _ => "/results",
    };
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let title = item.get("title")?.as_str()?;
            let url = item.get("url")?.as_str()?;
            let description = if source == "keenable" {
                item.get("snippet").and_then(Value::as_str)
            } else if source == "parallel" {
                item.get("excerpts")
                    .and_then(Value::as_array)
                    .and_then(|items| items.first())
                    .and_then(Value::as_str)
            } else {
                item.get("description")
                    .or_else(|| item.get("content"))
                    .and_then(Value::as_str)
            }
            .unwrap_or_default();
            Some(WebSource {
                title: title.into(),
                url: url.into(),
                description: description.into(),
            })
        })
        .collect()
}

fn parse_duckduckgo(html: &str) -> Vec<WebSource> {
    let mut remaining = html;
    let mut results = Vec::new();
    while let Some(index) = remaining.find("class=\"result__a\"") {
        remaining = &remaining[index..];
        let Some(open_end) = remaining.find('>') else {
            break;
        };
        let tag = &remaining[..open_end];
        let Some(close) = remaining[open_end + 1..].find("</a>") else {
            break;
        };
        let title = html_escape::decode_html_entities(&strip_tags(
            &remaining[open_end + 1..open_end + 1 + close],
        ))
        .to_string();
        let href = tag
            .split("href=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or_default();
        let decoded_href = html_escape::decode_html_entities(href);
        let url = Url::parse(&format!("https:{decoded_href}"))
            .ok()
            .and_then(|url| {
                url.query_pairs()
                    .find(|(key, _)| key == "uddg")
                    .map(|(_, value)| value.into_owned())
            })
            .unwrap_or_else(|| decoded_href.to_string());
        remaining = &remaining[open_end + 1 + close + 4..];
        let section_end = remaining
            .find("class=\"result__a\"")
            .unwrap_or(remaining.len());
        let section = &remaining[..section_end];
        let description = section
            .find("class=\"result__snippet\"")
            .and_then(|index| {
                let after = &section[index..];
                let start = after.find('>')? + 1;
                let end = after[start..].find("</a>")? + start;
                Some(html_escape::decode_html_entities(&strip_tags(&after[start..end])).to_string())
            })
            .unwrap_or_default();
        results.push(WebSource {
            title,
            url,
            description,
        });
    }
    results
}

fn strip_tags(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ring_needs_no_key_and_brave_is_optional() {
        let settings = WebSearchSettings::default();
        for id in ["firecrawl", "keenable", "exa", "duckduckgo", "searxng"] {
            assert!(settings.source(id).unwrap().enabled);
        }
        assert!(!settings.source("brave").unwrap().enabled);
        assert!(!settings.source("parallel").unwrap().enabled);
    }

    #[test]
    fn parses_provider_payloads_and_rejects_unsafe_links() {
        let firecrawl = json!({"data":{"web":[{"title":"Docs","url":"https://example.org","description":"Good"},{"title":"Bad","url":"javascript:alert(1)"}]}});
        assert_eq!(
            clean_results(parse_json_results("firecrawl", &firecrawl)).len(),
            1
        );
        let html = r#"<a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.org&amp;rut=1">Example &amp; docs</a><a class="result__snippet">Useful <b>guide</b></a>"#;
        let result = clean_results(parse_duckduckgo(html));
        assert_eq!(result[0].url, "https://example.org");
        assert_eq!(result[0].title, "Example & docs");
        assert_eq!(result[0].description, "Useful guide");
        let exa = json!({"result":{"content":[{"text":"Title: Rust docs\nURL: https://doc.rust-lang.org/\nHighlights:\nOfficial guide\n---\nTitle: Other\nURL: https://example.com/\nHighlights:\nOverview"}]}});
        assert_eq!(clean_results(parse_json_results("exa", &exa)).len(), 2);
        let searxng = json!({"results":[{"title":"Search result","url":"https://example.org/","content":"Summary"}]});
        assert_eq!(
            clean_results(parse_json_results("searxng", &searxng))[0].description,
            "Summary"
        );
    }
}
