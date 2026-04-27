use thiserror::Error;

#[derive(Error, Debug)]
pub enum DockerError {
    #[error("Docker API: {0}")]
    Api(#[from] bollard::errors::Error),
    #[error("JSON: {0}")]
    Json(String),
    #[error("Stats: {0}")]
    Stats(String),
}
