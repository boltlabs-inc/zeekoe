use dialectic::{offer, types::*, Chan};
use tonic::transport::Server;
use zeekoe::wire::dynamic::{self, server};

// async fn echo(mut c: dynamic::server::Connection) -> Result<(), dynamic::Error> {
//     loop {
//         let x: String = c.recv().await?;
//         c.send(&x).await?;
//     }
// }

// type EchoServer = Loop<Recv<String, Send<String, Recur>>>;

// async fn echo_typed(
//     mut tx: server::ToClient,
//     mut rx: server::FromClient,
// ) -> Result<(), dynamic::Error> {
//     let chan: Chan<_, _, EchoServer> = Chan::new(&mut tx, &mut rx);
//     let mut chan = chan.enter();
//     loop {
//         //let x = "test".to_string();
//         let (x, c): (String, _) = chan.recv().await?;
//         let c = c.send(&x).await?;
//         chan = c.recur();
//     }
// }

type IntOrString = Offer<(Send<i64, End>, (Send<String, End>, ()))>;

async fn int_or_string(tx: server::ToClient, rx: server::FromClient) -> Result<(), dynamic::Error> {
    let chan: Chan<_, _, IntOrString> = Chan::new(tx, rx);
    offer!(chan =>
        chan.send(&1).await?,
        chan.send(&"test".to_string()).await?,
    )
    .close();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;

    Server::builder()
        .add_service(dynamic::server::DynamicServer::new(int_or_string))
        .serve(addr)
        .await?;

    Ok(())
}
