use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::Config;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use openraft::Vote;
use openraft_memstore::BlockOperation;

use crate::fixtures::blank_ent;
use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::log_id;
use crate::fixtures::RaftRouter;

/// When building a snapshot, append-entries request should not be blocked.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn building_snapshot_does_not_block_append() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    let mut log_index = router.new_cluster(btreeset! {0,1}, btreeset! {}).await?;

    let follower = router.get_raft_handle(&1)?;

    tracing::info!("--- set flag to block snapshot building");
    {
        let (mut _sto1, sm1) = router.get_storage_handle(&1)?;
        sm1.storage_mut().await.set_blocking(BlockOperation::BuildSnapshot, Duration::from_millis(5_000));
    }

    tracing::info!("--- build snapshot on follower, it should block");
    {
        log_index += router.client_request_many(0, "0", 10).await?;
        router.wait(&1, timeout()).log(Some(log_index), "written 10 logs").await?;

        follower.trigger_snapshot().await?;

        tracing::info!("--- sleep 500 ms to make sure snapshot is started");
        tokio::time::sleep(Duration::from_millis(500)).await;

        let res = router
            .wait(&1, Some(Duration::from_millis(500)))
            .snapshot(log_id(1, 0, log_index), "building snapshot is blocked")
            .await;
        assert!(res.is_err(), "snapshot should be blocked and can not finish");
    }

    tracing::info!("--- send append-entries request to the follower that is building snapshot");
    {
        let rpc = AppendEntriesRequest::<openraft_memstore::Config> {
            vote: Vote::new_committed(1, 0),
            prev_log_id: Some(log_id(1, 0, log_index)),
            entries: vec![blank_ent(1, 15)],
            leader_commit: None,
        };

        let mut cli = router.new_client(1, &()).await;
        let fu = cli.send_append_entries(rpc);
        let fu = tokio::time::timeout(Duration::from_millis(500), fu);
        let resp = fu.await??;
        assert!(resp.is_success());
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}