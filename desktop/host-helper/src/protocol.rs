use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostEntryInfo {
    pub domain: String,
    pub ip: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum HelperRequest {
    Add {
        domain: String,
        #[serde(default)]
        ip: Option<String>,
    },
    Remove {
        domain: String,
    },
    Ping,
    List,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<HostEntryInfo>>,
}

impl HelperResponse {
    pub fn success(msg: impl Into<String>) -> Self {
        Self {
            success: true,
            message: Some(msg.into()),
            error: None,
            entries: None,
        }
    }

    pub fn error(err: impl Into<String>) -> Self {
        Self {
            success: false,
            message: None,
            error: Some(err.into()),
            entries: None,
        }
    }

    pub fn entries(entries: Vec<HostEntryInfo>) -> Self {
        Self {
            success: true,
            message: Some(format!("Found {} entries", entries.len())),
            error: None,
            entries: Some(entries),
        }
    }
}
