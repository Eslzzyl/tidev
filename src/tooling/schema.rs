use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

pub trait ToolArgs: Sized + Clone + std::fmt::Debug + Serialize + DeserializeOwned {
    fn schema() -> Value;
}
