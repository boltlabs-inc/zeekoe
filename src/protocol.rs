use dialectic::prelude::*;

pub type Ping = Session! {
    recv String;
    send String;
};
