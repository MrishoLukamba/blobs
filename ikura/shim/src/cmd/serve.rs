use crate::{cli::serve::Params, dock, ikura_rpc::Client};
use jsonrpsee::server::Server;
use tracing::info;

pub async fn run(params: Params) -> anyhow::Result<()> {
    info!(
        "starting ikura-shim server on {}:{}",
        params.dock.address, params.dock.port
    );
    let listen_on = (params.dock.address.as_str(), params.dock.port);
    let submit_key = crate::cmd::load_key(params.key_management)?;
    if submit_key.is_none() {
        tracing::info!(
            "no submit key provided, will not be able to submit blobs. \
Pass --submit-dev-alice or --submit-private-key=<..> to fix."
        );
    }
    let server = Server::builder().build(listen_on).await?;
    let client = connect_client(&params.rpc.node_url, params.rpc.no_retry).await?;
    let methods = dock::init(dock::Config {
        // TODO: whenever there are more docks, the logic of checking if any at least one is enabled
        //       and other similar stuff should be in CLI.
        enable_sovereign: params.dock.enable_sovereign(),
        enable_rollkit: params.dock.enable_rollkit(),
        client,
        submit_key,
    });
    let handle = server.start(methods);
    handle.stopped().await;
    Ok(())
}

async fn connect_client(url: &str, no_retry: bool) -> anyhow::Result<Client> {
    let client = Client::new(url.to_string(), no_retry).await?;
    Ok(client)
}