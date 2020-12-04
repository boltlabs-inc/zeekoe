use dialectic::{types::*, Chan, Session};
use zeekoe::wire::dynamic::client;

type IntOrString = Offer<(Send<i64, End>, (Send<String, End>, ()))>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "http://localhost:50051".parse::<tonic::transport::Uri>()?;
    let (tx, rx) = client::connect(addr).await?;
    let chan: Chan<_, _, <IntOrString as Session>::Dual> = Chan::new(tx, rx);
    let chan = chan.choose::<S<Z>>().await?;
    let (x, _) = chan.recv().await?;
    println!("Received: {}", x);
    // loop {
    //     let mut line = String::new();
    //     std::io::stdin().read_line(&mut line)?;
    //     if line == "" {
    //         break;
    //     } else {
    //         client.send(&line).await?;
    //         let response: String = client.recv().await?;
    //         print!("Response: {}", response);
    //     }
    // }

    Ok(())
}
