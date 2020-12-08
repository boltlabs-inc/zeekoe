#![allow(unused)]
use dialectic::{loop_, offer, types::*, Chan, Session, Val};
use zeekoe::wire::dynamic::{client, Error};

type IntOrString = Offer<(Send<i64, End>, (Send<String, End>, ()))>;

type EchoServer = Loop<Recv<String, Choose<(Send<String, Recur>, (End, ()))>>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "http://localhost:50051".parse::<tonic::transport::Uri>()?;
    let (tx, rx) = client::connect(addr).await?;
    let c: Chan<_, _, <EchoServer as Session>::Dual> = Chan::new(tx, rx);
    loop_! { c =>
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        let c = c.send::<Val>(line).await?;
        offer! { c =>
            {
                let (response, c): (String, _) = c.recv().await?;
                print!("Response: {}", response);
                c
            },
            break,
            ? => Err(Error::Disconnected)?,
        }
    }
    Ok(())
}
