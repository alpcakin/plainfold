// A model behind an OpenAI-style chat completions endpoint: OpenAI itself, and the many services and local servers
// that answer the same requests. The model gets no tools and its answer is held to a JSON shape.
use super::{Answer, Fail, Provider};
use serde_json::{Value, json};
use std::time::Duration;

pub const URL: &str = "https://api.openai.com/v1/chat/completions";
pub const MODEL: &str = "gpt-4o-mini";

pub struct OpenAi {
    url: String,
    key: String,
    model: String,
    agent: ureq::Agent,
}

impl OpenAi {
    /// The model and the endpoint: `PLAINFOLD_LLM_MODEL` and `PLAINFOLD_LLM_URL` choose others, for development.
    pub fn with_key(key: &str) -> Self {
        let env = |k: &str, d: &str| std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string());
        OpenAi::new(key, &env("PLAINFOLD_LLM_MODEL", MODEL), &env("PLAINFOLD_LLM_URL", URL))
    }

    pub fn new(key: &str, model: &str, url: &str) -> Self {
        let config = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(120))).build();
        OpenAi { url: url.to_string(), key: key.to_string(), model: model.to_string(), agent: config.into() }
    }
}

/// The answer: one text per step, by number.
fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "steps": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": { "n": { "type": "integer" }, "text": { "type": "string" } },
                    "required": ["n", "text"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["steps"],
        "additionalProperties": false
    })
}

impl Provider for OpenAi {
    fn model(&self) -> String {
        self.model.clone()
    }

    fn ask(&self, system: &str, user: &str) -> Result<Answer, Fail> {
        let body = json!({
            "model": self.model,
            "temperature": 0,
            "seed": 1,
            "max_tokens": 2000,
            "messages": [{ "role": "system", "content": system }, { "role": "user", "content": user }],
            "response_format": { "type": "json_schema", "json_schema": { "name": "labels", "strict": true, "schema": schema() } }
        });
        let mut resp = self
            .agent
            .post(&self.url)
            .header("Authorization", &format!("Bearer {}", self.key))
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .map_err(|e| Fail::Retry(format!("could not reach {}: {e}", host(&self.url)), None))?;
        let status = resp.status().as_u16();
        let after = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<f64>().ok()).map(Duration::from_secs_f64);
        let text = resp.body_mut().read_to_string().map_err(|e| Fail::Retry(e.to_string(), None))?;
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let message = v["error"]["message"].as_str().unwrap_or("").to_string();
        match status {
            200 => {}
            401 | 403 => return Err(Fail::Key(format!("{} refused the key ({status}){}", host(&self.url), said(&message)))),
            429 if v["error"]["code"] == "insufficient_quota" => return Err(Fail::Key(format!("the account has no credit left{}", said(&message)))),
            404 => return Err(Fail::Key(format!("{} has no model {}{}", host(&self.url), self.model, said(&message)))),
            429 | 500..=599 => return Err(Fail::Retry(format!("{} is busy ({status})", host(&self.url)), after)),
            _ => return Err(Fail::Other(format!("{} answered {status}{}", host(&self.url), said(&message)))),
        }
        let msg = &v["choices"][0]["message"];
        if let Some(r) = msg["refusal"].as_str() {
            return Err(Fail::Other(format!("the model refused: {r}")));
        }
        let content = msg["content"].as_str().ok_or_else(|| Fail::Other("the answer had no text".into()))?;
        let parsed: Value = serde_json::from_str(content).map_err(|_| Fail::Other("the answer was not in the asked shape".into()))?;
        let mut texts = vec![];
        for s in parsed["steps"].as_array().into_iter().flatten() {
            if let (Some(n), Some(t)) = (s["n"].as_u64(), s["text"].as_str()) {
                texts.push((n as usize, t.to_string()));
            }
        }
        let used = |k: &str| v["usage"][k].as_u64().unwrap_or(0);
        Ok(Answer { texts, tokens_in: used("prompt_tokens"), tokens_out: used("completion_tokens") })
    }
}

fn host(url: &str) -> &str {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url)
}

fn said(message: &str) -> String {
    if message.is_empty() { String::new() } else { format!(": {}", message.chars().take(200).collect::<String>()) }
}
