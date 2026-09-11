use winluks::{Error, adapter::check_consumer, probe::Filesystem};

#[test]
fn ext4_g0_failure_blocks_publication_before_driver_or_console_access() {
    assert_eq!(check_consumer(Filesystem::Ext4), Err(Error::FsGateUnpassed));
}
