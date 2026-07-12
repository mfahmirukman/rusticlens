use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("kubeconfig: {0}")]
    Kubeconfig(#[from] kube::config::KubeconfigError),

    #[error("kubernetes API: {0}")]
    Api(#[from] kube::Error),

    #[error("serialization: {0}")]
    Serialization(#[from] serde_yaml::Error),

    #[error("no active cluster context")]
    NoActiveContext,

    #[error("resource not found: {kind}/{name} in namespace {namespace}")]
    NotFound {
        kind: String,
        name: String,
        namespace: String,
    },

    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn user_message(&self) -> String {
        match self {
            Error::Api(kube::Error::Api(response)) if response.code == 401 => {
                "Authentication failed. Check your kubeconfig credentials.".to_string()
            }
            Error::Api(kube::Error::Api(response)) if response.code == 403 => {
                format!("Permission denied: {}", response.message.trim())
            }
            Error::Api(kube::Error::Api(response)) if response.code == 404 => {
                "Resource not found.".to_string()
            }
            other => other.to_string(),
        }
    }
}
