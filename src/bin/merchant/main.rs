#![allow(unused)]
use dialectic::{for_, loop_, offer, types::*, Chan, NewSession, Ref, Val};
use tonic::transport::Server;
use zeekoe::wire::dynamic::{self, server};

// async fn echo(mut c: dynamic::server::Connection) -> Result<(), dynamic::Error> {
//     loop {
//         let x: String = c.recv().await?;
//         c.send(&x).await?;
//     }
// }

type EchoServer = Loop<Recv<String, Choose<(Send<String, Recur>, (End, ()))>>>;

async fn echo_server(
    mut tx: server::ToClient,
    mut rx: server::FromClient,
) -> Result<(), dynamic::Error> {
    let c = EchoServer::wrap(&mut tx, &mut rx);
    let xs: [u8; 3] = [1, 2, 3];
    let c = for_! { _ in &xs, c =>
        let (x, c): (String, _) = c.recv().await?;
        // if x == "\n" {
        //     break;
        // } else {
            let c = c.choose::<Z>().await?;
            c.send::<Val>(x).await?
        // }
    };
    let (r, c) = c.recv().await?;
    c.choose::<S<Z>>().await?.close();
    Ok(())
}

type IntOrString = Offer<(Send<i64, End>, (Send<String, End>, ()))>;

async fn int_or_string(tx: server::ToClient, rx: server::FromClient) -> Result<(), dynamic::Error> {
    let chan = IntOrString::wrap(tx, rx);
    offer!(chan =>
        chan.send::<Val>(1).await?,
        chan.send::<Ref>(&"test".to_string()).await?,
        ? => panic!(),
    )
    .close();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;

    Server::builder()
        .add_service(dynamic::server::DynamicServer::new(echo_server))
        .serve(addr)
        .await?;

    Ok(())
}
