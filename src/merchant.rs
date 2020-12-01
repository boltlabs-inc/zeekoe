use tonic::transport::Server;

use crate::wire::{self, Bidirectional, Receive, Transmit};

async fn echo(mut c: wire::server::Connection) -> Result<(), wire::Error> {
    loop {
        let x = c.recv::<String>().await?;
        c.send(&x).await?;
    }
}

async fn splitted(mut c: wire::server::Connection) -> Result<(), wire::Error> {
    let (tx, rx) = c.split();
    let send_loop = async { while tx.send(&"Hello!").await.is_ok() {} };
    let recv_loop = async {
        while let Ok(m) = rx.recv::<String>().await {
            println!("{}", m)
        }
    };
    tokio::select! {
        _ = send_loop => {},
        _ = recv_loop => {},
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;

    Server::builder()
        .add_service(wire::GenericServer::new(splitted))
        .serve(addr)
        .await?;

    Ok(())
}
