tonic::include_proto!("wire");

pub(crate) mod establish {
    tonic::include_proto!("wire.establish");
}

pub(crate) mod activate {
    tonic::include_proto!("wire.activate");
}

pub(crate) mod pay {
    tonic::include_proto!("wire.pay");
}