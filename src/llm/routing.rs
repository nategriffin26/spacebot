//! Model routing configuration and resolution.

use crate::ProcessType;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct PromptRoutingConfig {
    pub enabled: bool,
    pub process_types: Vec<ProcessType>,
    pub tiers: PromptRoutingTiers,
    pub boundaries: PromptRoutingBoundaries,
    pub weights: PromptRoutingWeights,
}

impl Default for PromptRoutingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            process_types: vec![ProcessType::Channel, ProcessType::Branch],
            tiers: PromptRoutingTiers::default(),
            boundaries: PromptRoutingBoundaries::default(),
            weights: PromptRoutingWeights::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PromptRoutingTiers {
    pub light: Option<String>,
    pub standard: Option<String>,
    pub heavy: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct PromptRoutingBoundaries {
    pub light_standard: f32,
    pub standard_heavy: f32,
}

impl Default for PromptRoutingBoundaries {
    fn default() -> Self {
        Self {
            light_standard: 0.8,
            standard_heavy: 2.2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PromptRoutingWeights {
    pub token_count: f32,
    pub code_presence: f32,
    pub reasoning: f32,
    pub technical: f32,
    pub constraints: f32,
    pub multi_step: f32,
    pub simple: f32,
}

impl Default for PromptRoutingWeights {
    fn default() -> Self {
        Self {
            token_count: 0.5,
            code_presence: 0.7,
            reasoning: 0.8,
            technical: 0.5,
            constraints: 0.6,
            multi_step: 0.7,
            simple: -0.9,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptRoutingTier {
    Light,
    Standard,
    Heavy,
}

impl PromptRoutingTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Standard => "standard",
            Self::Heavy => "heavy",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoutingDecision<'a> {
    pub model_name: &'a str,
    pub prompt_tier: Option<PromptRoutingTier>,
    pub prompt_score: Option<f32>,
    pub resolution_reason: &'static str,
}

/// Model routing configuration. Lives on the agent config (via defaults).
/// Determines which LLM model each process type uses, with task-type
/// overrides for workers/branches and fallback chains for resilience.
#[derive(Debug, Clone)]
pub struct RoutingConfig {
    /// Model per process type.
    pub channel: String,
    pub branch: String,
    pub worker: String,
    pub compactor: String,
    pub cortex: String,
    pub voice: String,

    /// Task-type overrides (e.g. "coding" → "anthropic/claude-sonnet-4").
    /// Applied to workers and branches when a task_type is specified at spawn.
    pub task_overrides: HashMap<String, String>,

    /// Fallback chains per model. When a model fails with a retriable error,
    /// try the next model in its chain.
    pub fallbacks: HashMap<String, Vec<String>>,

    /// How long to deprioritize a rate-limited model (seconds).
    pub rate_limit_cooldown_secs: u64,

    pub channel_thinking_effort: String,
    pub branch_thinking_effort: String,
    pub worker_thinking_effort: String,
    pub compactor_thinking_effort: String,
    pub cortex_thinking_effort: String,
    pub prompt_routing: PromptRoutingConfig,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self::for_model("anthropic/claude-sonnet-4".into())
    }
}

impl RoutingConfig {
    /// Create a routing config that uses a single model for all process types.
    fn for_model(model: String) -> Self {
        Self {
            channel: model.clone(),
            branch: model.clone(),
            worker: model.clone(),
            compactor: model.clone(),
            cortex: model,
            voice: String::new(),
            task_overrides: HashMap::new(),
            fallbacks: HashMap::new(),
            rate_limit_cooldown_secs: 60,
            channel_thinking_effort: "auto".into(),
            branch_thinking_effort: "auto".into(),
            worker_thinking_effort: "auto".into(),
            compactor_thinking_effort: "auto".into(),
            cortex_thinking_effort: "auto".into(),
            prompt_routing: PromptRoutingConfig::default(),
        }
    }
}

impl RoutingConfig {
    /// Resolve the model name for a process type and optional task type.
    pub fn resolve(&self, process_type: ProcessType, task_type: Option<&str>) -> &str {
        self.resolve_with_prompt(process_type, task_type, None)
            .model_name
    }

    pub fn resolve_with_prompt<'a>(
        &'a self,
        process_type: ProcessType,
        task_type: Option<&str>,
        prompt_text: Option<&str>,
    ) -> RoutingDecision<'a> {
        // Check task-type override first (only for workers and branches)
        if let Some(task) = task_type
            && matches!(process_type, ProcessType::Worker | ProcessType::Branch)
            && let Some(override_model) = self.task_overrides.get(task)
        {
            return RoutingDecision {
                model_name: override_model,
                prompt_tier: None,
                prompt_score: None,
                resolution_reason: "task_override",
            };
        }

        if let Some(task) = task_type
            && matches!(process_type, ProcessType::Worker | ProcessType::Branch)
            && matches!(
                task,
                "source_verification"
                    | "artifact_grounded_copy"
                    | "capability_diagnosis"
                    | "multi_lane_synthesis"
            )
        {
            return RoutingDecision {
                model_name: &self.branch,
                prompt_tier: None,
                prompt_score: None,
                resolution_reason: "task_class_bias",
            };
        }

        if let Some(prompt_text) = prompt_text
            && let Some((tier, score)) = self.classify_prompt_tier(prompt_text, process_type)
            && let Some(model_name) = self.model_for_prompt_tier(tier, process_type)
        {
            return RoutingDecision {
                model_name,
                prompt_tier: Some(tier),
                prompt_score: Some(score),
                resolution_reason: "prompt_routing",
            };
        }

        let model_name = match process_type {
            ProcessType::Channel => &self.channel,
            ProcessType::Branch => &self.branch,
            ProcessType::Worker => &self.worker,
            ProcessType::Compactor => &self.compactor,
            ProcessType::Cortex => &self.cortex,
        };

        RoutingDecision {
            model_name,
            prompt_tier: None,
            prompt_score: None,
            resolution_reason: "process_default",
        }
    }

    pub fn thinking_effort_for_model(&self, model_name: &str) -> &str {
        if self.channel == model_name {
            return &self.channel_thinking_effort;
        }
        if self.branch == model_name {
            return &self.branch_thinking_effort;
        }
        if self.worker == model_name {
            return &self.worker_thinking_effort;
        }
        if self.compactor == model_name {
            return &self.compactor_thinking_effort;
        }
        if self.cortex == model_name {
            return &self.cortex_thinking_effort;
        }
        "auto"
    }

    /// Get the fallback chain for a model, if any.
    pub fn get_fallbacks(&self, model_name: &str) -> &[String] {
        self.fallbacks
            .get(model_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn classify_prompt_tier(
        &self,
        prompt_text: &str,
        process_type: ProcessType,
    ) -> Option<(PromptRoutingTier, f32)> {
        let config = &self.prompt_routing;
        if !config.enabled || !config.process_types.contains(&process_type) {
            return None;
        }

        let lower = prompt_text.to_ascii_lowercase();
        let weights = config.weights;
        let mut score = score_token_count(prompt_text) * weights.token_count;
        score += score_keywords(&lower, CODE_KEYWORDS) * weights.code_presence;
        score += score_keywords(&lower, REASONING_KEYWORDS) * weights.reasoning;
        score += score_keywords(&lower, TECHNICAL_KEYWORDS) * weights.technical;
        score += score_keywords(&lower, CONSTRAINT_KEYWORDS) * weights.constraints;
        score += score_multi_step(&lower) * weights.multi_step;
        score += score_keywords(&lower, SIMPLE_KEYWORDS) * weights.simple;

        let tier = if score < config.boundaries.light_standard {
            PromptRoutingTier::Light
        } else if score >= config.boundaries.standard_heavy {
            PromptRoutingTier::Heavy
        } else {
            PromptRoutingTier::Standard
        };

        Some((tier, score))
    }

    fn model_for_prompt_tier(
        &self,
        tier: PromptRoutingTier,
        process_type: ProcessType,
    ) -> Option<&str> {
        let default_model = match process_type {
            ProcessType::Channel => self.channel.as_str(),
            ProcessType::Branch => self.branch.as_str(),
            ProcessType::Worker => self.worker.as_str(),
            ProcessType::Compactor => self.compactor.as_str(),
            ProcessType::Cortex => self.cortex.as_str(),
        };

        match tier {
            PromptRoutingTier::Light => self
                .prompt_routing
                .tiers
                .light
                .as_deref()
                .or(Some(default_model)),
            PromptRoutingTier::Standard => self
                .prompt_routing
                .tiers
                .standard
                .as_deref()
                .or(Some(default_model)),
            PromptRoutingTier::Heavy => self
                .prompt_routing
                .tiers
                .heavy
                .as_deref()
                .or(Some(default_model)),
        }
    }
}

const CODE_KEYWORDS: &[&str] = &[
    "code",
    "rust",
    "python",
    "typescript",
    "compile",
    "test",
    "debug",
    "refactor",
    "build",
    "implementation",
];
const REASONING_KEYWORDS: &[&str] = &[
    "why",
    "analyze",
    "compare",
    "tradeoff",
    "deep dive",
    "root cause",
    "investigate",
    "verify",
];
const TECHNICAL_KEYWORDS: &[&str] = &[
    "api",
    "config",
    "schema",
    "migration",
    "architecture",
    "capability",
    "memory",
    "worker",
];
const CONSTRAINT_KEYWORDS: &[&str] = &[
    "must", "should", "without", "do not", "never", "exactly", "fully",
];
const SIMPLE_KEYWORDS: &[&str] = &["hi", "hello", "thanks", "what time", "quickly", "simple"];
const MULTI_STEP_HINTS: &[&str] = &["1.", "2.", "first", "then", "after that", "finally"];

fn score_keywords(lower: &str, keywords: &[&str]) -> f32 {
    keywords
        .iter()
        .filter(|keyword| lower.contains(**keyword))
        .count() as f32
}

fn score_multi_step(lower: &str) -> f32 {
    score_keywords(lower, MULTI_STEP_HINTS).min(3.0)
}

fn score_token_count(text: &str) -> f32 {
    let tokens = text.split_whitespace().count();
    match tokens {
        0..=24 => 0.0,
        25..=79 => 0.7,
        80..=199 => 1.3,
        _ => 2.0,
    }
}

/// Whether an HTTP status code should trigger a fallback to the next model.
pub fn is_retriable_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

/// Whether a completion error message indicates a retriable failure.
pub fn is_retriable_error(error_message: &str) -> bool {
    let lower = error_message.to_lowercase();
    // Rate limits and server errors
    lower.contains("429")
        || lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("rate limit")
        || lower.contains("overloaded")
        || lower.contains("timeout")
        || lower.contains("connection")
        // Generic server errors (OpenRouter wraps upstream 500s in various
        // phrasings like "The server had an error while processing your request")
        || lower.contains("server error")
        || lower.contains("server had an error")
        || lower.contains("internal error")
        // Empty/malformed responses are transient provider issues
        || lower.contains("empty response")
        || lower.contains("empty or unsupported response")
        || lower.contains("failed to read response body")
        || lower.contains("error decoding response body")
}

/// Whether a completion error indicates context window overflow.
///
/// Providers return 400 with various phrasings when the request exceeds
/// the model's context limit. Checking for these lets workers compact
/// and retry instead of dying.
pub fn is_context_overflow_error(error_message: &str) -> bool {
    let lower = error_message.to_lowercase();
    lower.contains("context length")
        || lower.contains("maximum context")
        || lower.contains("token limit")
        || lower.contains("too many tokens")
        || lower.contains("request too large")
        || lower.contains("content_too_large")
        || lower.contains("max_tokens")
        || (lower.contains("maximum") && lower.contains("tokens"))
}

/// Returns routing defaults appropriate for a given provider.
///
/// When a user sets up OpenRouter but routing still points to `anthropic/...`,
/// every LLM call fails because there's no Anthropic key. This function gives
/// each provider sane defaults so things work out of the box.
pub fn defaults_for_provider(provider: &str) -> RoutingConfig {
    match provider {
        "anthropic" => RoutingConfig::for_model("anthropic/claude-sonnet-4".into()),
        "openrouter" => {
            let channel: String = "openrouter/anthropic/claude-sonnet-4-20250514".into();
            let worker: String = "openrouter/anthropic/claude-haiku-4.5-20250514".into();
            RoutingConfig {
                channel: "openrouter/anthropic/claude-sonnet-4-20250514".into(),
                branch: "openrouter/anthropic/claude-sonnet-4-20250514".into(),
                worker: "openrouter/anthropic/claude-haiku-4.5-20250514".into(),
                compactor: "openrouter/anthropic/claude-haiku-4.5-20250514".into(),
                cortex: "openrouter/anthropic/claude-haiku-4.5-20250514".into(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::from([(channel, vec![worker])]),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "kilo" => {
            let channel: String = "kilo/anthropic/claude-sonnet-4.5".into();
            let worker: String = "kilo/anthropic/claude-haiku-4.5".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker,
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel)]),
                fallbacks: HashMap::new(),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "openai" => {
            let channel: String = "openai/gpt-4.1".into();
            let worker: String = "openai/gpt-4.1-mini".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker.clone(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::from([(channel, vec![worker])]),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "openai-chatgpt" => {
            let channel: String = "openai-chatgpt/gpt-5.4".into();
            let worker: String = "openai-chatgpt/gpt-5-mini".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker.clone(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::from([(channel, vec![worker])]),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "zhipu" => {
            let channel: String = "zhipu/glm-4-plus".into();
            let worker: String = "zhipu/glm-4-flash".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker.clone(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::from([(channel, vec![worker])]),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "groq" => {
            let channel: String = "groq/llama-3.3-70b-versatile".into();
            let worker: String = "groq/llama-3.3-70b-specdec".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker.clone(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::from([(channel, vec![worker])]),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "together" => {
            let channel: String = "together/meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo".into();
            let worker: String = "together/meta-llama/Meta-Llama-3.1-70B-Instruct-Turbo".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker.clone(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::from([(channel, vec![worker])]),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "fireworks" => {
            let channel: String =
                "fireworks/accounts/fireworks/models/llama-v3p3-70b-instruct".into();
            let worker: String =
                "fireworks/accounts/fireworks/models/llama-v3p1-8b-instruct".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker.clone(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::from([(channel, vec![worker])]),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "deepseek" => {
            let channel: String = "deepseek/deepseek-chat".into();
            let worker: String = "deepseek/deepseek-chat".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker.clone(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::new(),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "xai" => {
            let channel: String = "xai/grok-2-latest".into();
            let worker: String = "xai/grok-2-latest".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker.clone(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::new(),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "mistral" => {
            let channel: String = "mistral/mistral-large-latest".into();
            let worker: String = "mistral/mistral-small-latest".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker.clone(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::from([(channel, vec![worker])]),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "gemini" => {
            let channel: String = "gemini/gemini-2.5-pro".into();
            let worker: String = "gemini/gemini-2.5-flash".into();
            let lite: String = "gemini/gemini-2.5-flash-lite".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker.clone(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::from([(channel, vec![worker.clone()]), (worker, vec![lite])]),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "opencode-zen" => {
            let channel: String = "opencode-zen/kimi-k2.5".into();
            let worker: String = "opencode-zen/kimi-k2.5".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker.clone(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::new(),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "opencode-go" => {
            let channel: String = "opencode-go/kimi-k2.5".into();
            let worker: String = "opencode-go/kimi-k2".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker.clone(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::from([(channel, vec![worker])]),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        "nvidia" => RoutingConfig::for_model("nvidia/meta/llama-3.1-405b-instruct".into()),
        "minimax" => RoutingConfig::for_model("minimax/MiniMax-M2.5".into()),
        "minimax-cn" => RoutingConfig::for_model("minimax-cn/MiniMax-M2.5".into()),
        "moonshot" => RoutingConfig::for_model("moonshot/kimi-k2.5".into()),
        "zai-coding-plan" => RoutingConfig::for_model("zai-coding-plan/glm-5".into()),
        "github-copilot" => {
            let channel: String = "github-copilot/claude-sonnet-4".into();
            let worker: String = "github-copilot/gpt-4.1-mini".into();
            RoutingConfig {
                channel: channel.clone(),
                branch: channel.clone(),
                worker: worker.clone(),
                compactor: worker.clone(),
                cortex: worker.clone(),
                voice: String::new(),
                task_overrides: HashMap::from([("coding".into(), channel.clone())]),
                fallbacks: HashMap::from([(channel, vec![worker])]),
                rate_limit_cooldown_secs: 60,
                ..RoutingConfig::default()
            }
        }
        // Unknown — use the standard defaults
        _ => RoutingConfig::default(),
    }
}

/// Maps a provider ID to the prefix used in model routing strings.
pub fn provider_to_prefix(provider: &str) -> &str {
    match provider {
        "openrouter" => "openrouter/",
        "kilo" => "kilo/",
        "openai" => "openai/",
        "openai-chatgpt" => "openai-chatgpt/",
        "anthropic" => "anthropic/",
        "zhipu" => "zhipu/",
        "groq" => "groq/",
        "together" => "together/",
        "fireworks" => "fireworks/",
        "deepseek" => "deepseek/",
        "xai" => "xai/",
        "mistral" => "mistral/",
        "gemini" => "gemini/",
        "nvidia" => "nvidia/",
        "opencode-zen" => "opencode-zen/",
        "opencode-go" => "opencode-go/",
        "minimax" => "minimax/",
        "minimax-cn" => "minimax-cn/",
        "moonshot" => "moonshot/",
        "zai-coding-plan" => "zai-coding-plan/",
        "github-copilot" => "github-copilot/",
        _ => "",
    }
}

/// Extracts the provider from a model routing string.
pub fn provider_from_model(model: &str) -> &str {
    if let Some((provider, _)) = model.split_once('/') {
        provider
    } else {
        "anthropic"
    }
}

/// Max number of fallback models to try before giving up.
pub const MAX_FALLBACK_ATTEMPTS: usize = 3;

/// Max retries per model (primary or fallback) on retriable errors.
pub const MAX_RETRIES_PER_MODEL: usize = 3;

/// Base delay for exponential backoff between retries (milliseconds).
pub const RETRY_BASE_DELAY_MS: u64 = 500;

/// Whether an error indicates an actual rate limit (429) vs other transient failures.
/// Only rate-limit errors should trigger cooldown — timeouts and 5xx errors are
/// momentary and shouldn't lock out a model for the full cooldown period.
pub fn is_rate_limit_error(error_message: &str) -> bool {
    let lower = error_message.to_lowercase();
    lower.contains("429") || lower.contains("rate limit")
}

#[cfg(test)]
mod tests {
    use super::defaults_for_provider;

    #[test]
    fn openai_chatgpt_defaults_use_gpt_5_4_primary_model() {
        let routing = defaults_for_provider("openai-chatgpt");

        assert_eq!(routing.channel, "openai-chatgpt/gpt-5.4");
        assert_eq!(routing.branch, "openai-chatgpt/gpt-5.4");
        assert_eq!(routing.worker, "openai-chatgpt/gpt-5-mini");
        assert_eq!(routing.compactor, "openai-chatgpt/gpt-5-mini");
        assert_eq!(routing.cortex, "openai-chatgpt/gpt-5-mini");
        assert_eq!(
            routing.task_overrides.get("coding").map(String::as_str),
            Some("openai-chatgpt/gpt-5.4")
        );
        assert_eq!(
            routing
                .fallbacks
                .get("openai-chatgpt/gpt-5.4")
                .map(Vec::as_slice),
            Some(&["openai-chatgpt/gpt-5-mini".to_string()][..])
        );
    }
}
