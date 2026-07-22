use anyhow::{Context, bail};
use tracing_subscriber::EnvFilter;

const DEFAULT_FILTER: &str = "labello_server=info,labello_api=info,labello_storage=info";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogFormat {
    Text,
    Json,
}

pub fn init() -> anyhow::Result<()> {
    let filter = filter(std::env::var("RUST_LOG").ok().as_deref())?;
    match log_format(std::env::var("LABELLO_LOG_FORMAT").ok().as_deref())? {
        LogFormat::Text => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .try_init()
            .map_err(|error| anyhow::anyhow!("could not initialize text logging: {error}")),
        LogFormat::Json => tracing_subscriber::fmt()
            .json()
            .with_current_span(true)
            .with_env_filter(filter)
            .try_init()
            .map_err(|error| anyhow::anyhow!("could not initialize JSON logging: {error}")),
    }
}

fn filter(value: Option<&str>) -> anyhow::Result<EnvFilter> {
    EnvFilter::try_new(value.unwrap_or(DEFAULT_FILTER)).context("invalid RUST_LOG filter")
}

fn log_format(value: Option<&str>) -> anyhow::Result<LogFormat> {
    match value.unwrap_or("text") {
        "text" => Ok(LogFormat::Text),
        "json" => Ok(LogFormat::Json),
        value => bail!("invalid LABELLO_LOG_FORMAT {value:?}; expected text or json"),
    }
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::filter::LevelFilter;

    use super::*;

    #[test]
    fn default_filter_enables_application_info() {
        let filter = filter(None).unwrap();
        assert_eq!(filter.max_level_hint(), Some(LevelFilter::INFO));
    }

    #[test]
    fn explicit_filter_and_format_are_validated() {
        assert!(filter(Some("labello_api=debug")).is_ok());
        assert!(filter(Some("labello_api=[")).is_err());
        assert_eq!(log_format(Some("json")).unwrap(), LogFormat::Json);
        assert!(log_format(Some("pretty")).is_err());
    }
}
