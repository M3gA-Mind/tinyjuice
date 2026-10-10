//! Plain-text compressor.
//!
//! TextCrusher provides a deterministic extractive compression: it keeps
//! verbatim salient/query-relevant spans and suppresses near-duplicates.

use async_trait::async_trait;
use std::collections::HashSet;

use super::Compressor;
use crate::cache::CcrStore;
use crate::pipeline::{OffloadOutput, OffloadTransform, PipelineInput, estimate_bloat};
use crate::relevance::Bm25Corpus;
use crate::types::{CompressInput, CompressOptions, CompressOutput, CompressorKind, ContentKind};

pub const MIN_SEGMENTS: usize = 12;
pub const TARGET_RATIO: f32 = 0.40;
const MIN_TARGET_CHARS: usize = 800;

pub struct TextCompressor;

/// Typed offload transform for deterministic extractive TextCrusher.
#[derive(Debug, Clone)]
pub struct TextCrusherTransform {
    options: CompressOptions,
    query: Option<String>,
}

impl TextCrusherTransform {
    pub fn new(options: CompressOptions) -> Self {
        Self {
            options,
            query: None,
        }
    }

    pub fn with_query(mut self, query: impl Into<String>) -> Self {
        self.query = Some(query.into());
        self
    }

    pub fn options(&self) -> &CompressOptions {
        &self.options
    }

    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }
}

impl Default for TextCrusherTransform {
    fn default() -> Self {
        Self::new(CompressOptions::default())
    }
}

impl OffloadTransform for TextCrusherTransform {
    fn name(&self) -> &'static str {
        "textcrusher"
    }

    fn estimate_bloat(&self, input: &PipelineInput<'_>) -> f32 {
        if input.content_kind != ContentKind::PlainText {
            return 0.0;
        }
        let score = f32::from(estimate_bloat(input.content, input.content_kind).score) / 100.0;
        if self
            .query
            .as_deref()
            .is_some_and(|query| !query.trim().is_empty())
        {
            score.max(0.1)
        } else {
            score
        }
    }

    fn apply(&self, input: &PipelineInput<'_>, store: &dyn CcrStore) -> Option<OffloadOutput> {
        if input.content_kind != ContentKind::PlainText {
            return None;
        }
        let compacted = compress_textcrusher(input.content, self.query(), &self.options)?;
        OffloadOutput::from_retained_put(
            compacted.text,
            CompressorKind::TextCrusher,
            store.put(input.original_content),
        )
    }
}

#[async_trait]
impl Compressor for TextCompressor {
    fn kind(&self) -> CompressorKind {
        CompressorKind::TextCrusher
    }

    async fn compress(
        &self,
        input: &CompressInput<'_>,
        opts: &CompressOptions,
    ) -> Option<CompressOutput> {
        compress_textcrusher(input.content, input.hint.query.as_deref(), opts)
    }
}

#[derive(Debug, Clone)]
struct Segment {
    index: usize,
    text: String,
    terms: Vec<String>,
    score: f32,
    salience: f32,
}

pub fn compress_textcrusher(
    content: &str,
    query: Option<&str>,
    opts: &CompressOptions,
) -> Option<CompressOutput> {
    let mut segments = split_segments(content);
    if segments.len() < MIN_SEGMENTS {
        return None;
    }

    let duplicate_pressure = duplicate_pressure(&segments);
    let query = query.filter(|q| !q.trim().is_empty());
    let bm25 = query.map(|_| {
        Bm25Corpus::from_tokenized(
            segments
                .iter()
                .map(|segment| segment.terms.clone())
                .collect(),
        )
    });

    let segment_count = segments.len();
    for segment in &mut segments {
        segment.salience = salience_score(&segment.text);
        let recency = (segment.index + 1) as f32 / segment_count as f32;
        let query_score = match (query, bm25.as_ref()) {
            (Some(q), Some(corpus)) => corpus.score(q, segment.index).min(4.0) / 4.0,
            _ => 0.0,
        };
        segment.score = segment.salience * 2.0 + query_score * 2.5 + recency * 0.35;
    }

    let has_signal = query.is_some()
        || duplicate_pressure >= 4
        || segments.iter().any(|segment| segment.salience >= 0.5);
    if !has_signal {
        return None;
    }

    let ratio_target = (content.len() as f32 * TARGET_RATIO) as usize;
    let target_chars = opts
        .max_inline_chars
        .unwrap_or(ratio_target)
        .min(ratio_target.max(MIN_TARGET_CHARS))
        .min(content.len().saturating_sub(1));
    if target_chars == 0 {
        return None;
    }

    let mut ranked: Vec<usize> = (0..segments.len()).collect();
    ranked.sort_by(|&a, &b| {
        segments[b]
            .score
            .partial_cmp(&segments[a].score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(&b))
    });

    let mut selected = Vec::new();
    let mut selected_terms: Vec<HashSet<String>> = Vec::new();
    let mut used_chars = 0usize;

    for index in ranked {
        let segment = &segments[index];
        if used_chars + segment.text.len() + 1 > target_chars && !selected.is_empty() {
            continue;
        }
        let terms: HashSet<String> = segment.terms.iter().cloned().collect();
        if selected_terms
            .iter()
            .any(|existing| jaccard(existing, &terms) >= 0.82)
        {
            continue;
        }
        used_chars += segment.text.len() + 1;
        selected.push(index);
        selected_terms.push(terms);
    }

    if selected.is_empty() || selected.len() == segments.len() {
        return None;
    }

    selected.sort_unstable();
    let out = selected
        .into_iter()
        .map(|index| segments[index].text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    if out.len() >= content.len() {
        return None;
    }
    Some(CompressOutput::lossy(out, CompressorKind::TextCrusher))
}

fn split_segments(content: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    for block in content.split("\n\n") {
        let trimmed = block.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.len() <= 260 {
            push_segment(&mut segments, trimmed);
            continue;
        }
        for sentence in split_sentences(trimmed) {
            push_segment(&mut segments, sentence);
        }
    }
    segments
}

fn split_sentences(block: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut iter = block.char_indices().peekable();
    while let Some((idx, ch)) = iter.next() {
        let sentence_end = matches!(ch, '.' | '!' | '?')
            && iter.peek().is_none_or(|(_, next)| next.is_whitespace());
        if !sentence_end {
            continue;
        }
        let end = idx + ch.len_utf8();
        let sentence = block[start..end].trim();
        if !sentence.is_empty() {
            out.push(sentence);
        }
        while let Some((next_idx, next_ch)) = iter.peek().copied() {
            if next_ch.is_whitespace() {
                iter.next();
                start = next_idx + next_ch.len_utf8();
            } else {
                break;
            }
        }
    }
    let tail = block[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

fn push_segment(segments: &mut Vec<Segment>, text: &str) {
    let terms = crate::relevance::tokenize(text);
    if terms.len() < 3 {
        return;
    }
    segments.push(Segment {
        index: segments.len(),
        text: text.to_string(),
        terms,
        score: 0.0,
        salience: 0.0,
    });
}

fn salience_score(text: &str) -> f32 {
    let lower = text.to_ascii_lowercase();
    let mut score = 0.0f32;
    if lower.contains("error")
        || lower.contains("failed")
        || lower.contains("panic")
        || lower.contains("exception")
        || lower.contains("warning")
        || lower.contains("todo")
    {
        score += 1.0;
    }
    if text.chars().any(|ch| ch.is_ascii_digit()) {
        score += 0.35;
    }
    if text.split_whitespace().any(is_all_caps_identifier) {
        score += 0.5;
    }
    if text.split_whitespace().any(is_dotted_identifier) {
        score += 0.5;
    }
    score.min(2.0)
}

fn is_all_caps_identifier(token: &str) -> bool {
    let token = token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_');
    token.len() >= 4
        && token.chars().any(|ch| ch.is_ascii_alphabetic())
        && token
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}

fn is_dotted_identifier(token: &str) -> bool {
    let token = token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.');
    token.split('.').filter(|part| part.len() >= 2).count() >= 2
}

fn duplicate_pressure(segments: &[Segment]) -> usize {
    let mut seen: Vec<HashSet<String>> = Vec::new();
    let mut duplicates = 0usize;
    for segment in segments {
        let terms: HashSet<String> = segment.terms.iter().cloned().collect();
        if seen
            .iter()
            .any(|existing| jaccard(existing, &terms) >= 0.82)
        {
            duplicates += 1;
        } else {
            seen.push(terms);
        }
    }
    duplicates
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.len() + b.len() - intersection;
    intersection as f32 / union as f32
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
