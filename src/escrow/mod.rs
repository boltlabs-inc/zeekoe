pub mod notify;

pub mod types {

    use core::fmt;
    use serde::{Deserialize, Serialize};
    use std::fmt::{Display, Formatter};

    use tezedge::OriginatedAddress;

    /// Rename this type to match zkChannels written notation.
    /// Also, so we can easily change the tezedge type in case it is wrong.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct ContractId(OriginatedAddress);
    //pub type ContractId = OriginatedAddress;
    zkabacus_crypto::impl_sqlx_for_bincode_ty!(ContractId);

    impl Display for ContractId {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            // TODO: Fill in with actual contract ID
            std::fmt::Debug::fmt(self, f)
        }
    }

    impl ContractId {
        pub fn new(addr: OriginatedAddress) -> Self {
            Self(addr)
        }
    }
}
