use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use uuid::Uuid;

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

define_id!(NodeId);
define_id!(JobId);
define_id!(ChunkId);
define_id!(FileId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_serialize_and_deserialize_json() {
        let node_id = NodeId::new();
        let json = serde_json::to_string(&node_id).expect("serialize node id");
        let roundtrip: NodeId = serde_json::from_str(&json).expect("deserialize node id");
        assert_eq!(node_id, roundtrip);
    }
}
