
pub mod wire {
    tonic::include_proto!("wire");
    
    pub mod establish {
        tonic::include_proto!("establish");
    }
    
    pub mod activate {
        tonic::include_proto!("activate");
    }

    pub mod pay {
        tonic::include_proto!("pay");
    }
}