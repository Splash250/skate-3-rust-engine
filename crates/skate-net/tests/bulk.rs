use skate_net::bulk::{Channel, Limits};
#[test]
fn large_transfer_loss_duplicates_backpressure_and_cancellation() {
    let limits = Limits {
        max_message: 20_000,
        max_queued_bytes: 30_000,
        max_pending: 2,
    };
    let mut sender = Channel::new(limits).unwrap();
    let mut receiver = Channel::new(limits).unwrap();
    let bytes = (0..16_384).map(|i| (i % 251) as u8).collect::<Vec<_>>();
    let first = sender.send(bytes.clone()).unwrap();
    assert!(sender.send(bytes.clone()).is_err());
    let mut delivered = Vec::new();
    while let Some(frame) = sender.frame() {
        assert!(serde_json::to_vec(&frame).unwrap().len() <= 512);
        if let Some(data) = receiver.receive(&frame).unwrap() {
            delivered.push(data);
        }
        assert!(receiver.receive(&frame).unwrap().is_none());
        // Lost ACK is recovered by exactly the same frame and current cumulative ACK.
        sender.acknowledge(receiver.ack()).unwrap();
    }
    assert_eq!(delivered, vec![bytes.clone()]);
    assert_eq!(sender.queued_bytes(), 0);
    let second = sender.send(bytes).unwrap();
    assert!(second > first);
    receiver.receive(&sender.frame().unwrap()).unwrap();
    sender.cancel(second).unwrap();
    assert!(
        receiver
            .receive(&sender.frame().unwrap())
            .unwrap()
            .is_none()
    );
    sender.acknowledge(receiver.ack()).unwrap();
    assert!(sender.frame().is_none());
    sender.send(b"next".to_vec()).unwrap();
    assert_eq!(
        receiver.receive(&sender.frame().unwrap()).unwrap(),
        Some(b"next".to_vec())
    );
}
#[test]
fn forged_ack_and_oversized_or_out_of_order_chunk_do_not_advance() {
    let mut tx = Channel::new(Limits::default()).unwrap();
    let mut rx = Channel::new(Limits::default()).unwrap();
    tx.send(vec![1; 2048]).unwrap();
    let mut frame = tx.frame().unwrap();
    frame.total = usize::MAX;
    assert!(rx.receive(&frame).is_err());
    frame = tx.frame().unwrap();
    frame.offset = 192;
    assert!(rx.receive(&frame).is_err());
    assert!(
        tx.acknowledge(skate_net::bulk::Ack {
            id: 1,
            through: 2000,
            cancelled: false
        })
        .is_err()
    );
    assert_eq!(tx.frame().unwrap().offset, 0);
}

#[test]
fn progress_counts_only_acknowledged_bytes_and_reports_terminal_states() {
    use skate_net::bulk::Progress;
    let mut tx = Channel::default();
    let mut rx = Channel::default();
    let id = tx.send(vec![1; 1000]).unwrap();
    assert_eq!(tx.progress(id).unwrap(), Progress::Pending { acknowledged: 0, total: 1000 });
    rx.receive(&tx.frame().unwrap()).unwrap();
    assert_eq!(tx.progress(id).unwrap(), Progress::Pending { acknowledged: 0, total: 1000 });
    tx.acknowledge(rx.ack()).unwrap();
    assert_eq!(tx.progress(id).unwrap(), Progress::Pending { acknowledged: 192, total: 1000 });
    while let Some(frame) = tx.frame() {
        rx.receive(&frame).unwrap(); tx.acknowledge(rx.ack()).unwrap();
    }
    assert_eq!(tx.progress(id).unwrap(), Progress::Delivered { total: 1000 });
    let id = tx.send(vec![1; 999]).unwrap();
    tx.cancel(id).unwrap();
    assert_eq!(tx.progress(id).unwrap(), Progress::Cancelled);
    rx.receive(&tx.frame().unwrap()).unwrap(); tx.acknowledge(rx.ack()).unwrap();
    assert_eq!(tx.progress(id).unwrap(), Progress::Cancelled);
    assert!(tx.progress(id + 1).is_err());
}
