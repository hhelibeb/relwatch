mod calls;
mod config;
mod pipeline;
mod transport;

pub use config::read_config;
pub use pipeline::{
    generate_summaries_for_new, generate_translations_for_new, translate_single_release, ChunkSink,
};
pub use transport::{build_client, DEEPSEEK_TIMEOUT_SECS_TEST};
pub(crate) use transport::{chat_completion, count_prompt_chars};

#[cfg(test)]
mod tests;
