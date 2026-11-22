use std::{net::TcpListener, time::Duration};

use super::*;
use serde_json::{Value, json};
use tungstenite::Message;

fn read(socket: &mut tungstenite::WebSocket<std::net::TcpStream>) -> Value {
    loop {
        if let Message::Text(text) = socket.read().unwrap() {
            return serde_json::from_str(text.as_str()).unwrap();
        }
    }
}

fn send(socket: &mut tungstenite::WebSocket<std::net::TcpStream>, frame: &Value) {
    socket
        .send(Message::Text(frame.to_string().into()))
        .unwrap();
}

fn order(command: Command, n: u128) -> Order {
    Order {
        command,
        conn: ConnId(1),
        id: CommandId(uuid::Uuid::from_u128(n)),
    }
}

#[test]
fn a_clients_commands_reach_t3_and_its_answers_come_back_as_acks() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut socket = tungstenite::accept(stream).unwrap();
        send(
            &mut socket,
            &json!({"_tag": "Chunk", "requestId": "2", "values": [{"kind": "snapshot",
                "snapshotSequence": 1, "projection": {
                    "thread": {"id": "thread-1", "title": "Steer"},
                    "runs": [{"id": "run-1", "ordinal": 1, "status": "running"}]}}]}),
        );
        assert_eq!(read(&mut socket), json!({"_tag": "Ack", "requestId": "2"}));
        // The stop, as T3's own stop button sends it.
        let stop = read(&mut socket);
        assert_eq!(stop["tag"], "orchestration.dispatchCommand");
        assert_eq!(
            stop["payload"],
            json!({"type": "run.interrupt", "threadId": "thread-1", "runId": "run-1",
                "holdQueue": true,
                "commandId": "sheprd:00000000-0000-0000-0000-000000000001"})
        );
        send(
            &mut socket,
            &json!({"_tag": "Exit", "requestId": stop["id"],
                "exit": {"_tag": "Success", "value": {"sequence": 2}}}),
        );
        // A prompt T3 refuses.
        let prompt = read(&mut socket);
        assert_eq!(prompt["payload"]["type"], "message.dispatch");
        assert_eq!(prompt["payload"]["text"], "hi");
        send(
            &mut socket,
            &json!({"_tag": "Exit", "requestId": prompt["id"], "exit": {"_tag": "Failure",
                "cause": [{"_tag": "Fail", "error": "busy"}]}}),
        );
        let _ = socket.close(None);
        while socket.read().is_ok() {}
    });
    let stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let url = url::Url::parse(&format!("ws://127.0.0.1:{port}/ws")).unwrap();
    let connection = Connection::handshake(&url, stream.try_clone().unwrap()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    let (updates, changes) = bounded(1);
    let (orders, queue) = bounded(4);
    let worker = thread::spawn(move || {
        follow(
            connection,
            Follower::new("2".to_owned(), Vec::new()),
            &updates,
            &queue,
            Duration::from_secs(60),
        );
    });
    let next = || changes.recv_timeout(Duration::from_secs(10)).unwrap();
    let Step::Changed(transcript) = next() else {
        panic!("the snapshot first");
    };
    assert_eq!(transcript.status, Status::Working);
    assert!(!transcript.read_only);

    // A request that is not waiting is refused without reaching T3.
    let stale = Command::Answer {
        option: "accept".to_owned(),
        permission: sheprd_chat_protocol::PermissionId(5),
    };
    orders.send(order(stale, 9)).unwrap();
    assert_eq!(
        next(),
        Step::Acked {
            conn: ConnId(1),
            id: CommandId(uuid::Uuid::from_u128(9)),
            rejected: Some(Rejection::UnknownPermission),
        }
    );
    orders.send(order(Command::Cancel, 1)).unwrap();
    assert_eq!(
        next(),
        Step::Acked {
            conn: ConnId(1),
            id: CommandId(uuid::Uuid::from_u128(1)),
            rejected: None,
        }
    );
    let prompt = Command::Prompt {
        text: "hi".to_owned(),
        attachments: Vec::new(),
        origin: None,
    };
    orders.send(order(prompt, 2)).unwrap();
    assert_eq!(
        next(),
        Step::Acked {
            conn: ConnId(1),
            id: CommandId(uuid::Uuid::from_u128(2)),
            rejected: Some(Rejection::AgentUnavailable),
        }
    );
    assert!(matches!(next(), Step::Ended(_)));
    worker.join().unwrap();
    // The server reads until the socket closes.
    drop(stream);
    server.join().unwrap();
}

#[test]
fn a_command_before_the_snapshot_is_not_ready() {
    let follower = Follower::new("2".to_owned(), Vec::new());
    assert_eq!(
        follower.dispatch(&order(Command::Cancel, 1)),
        Err(Rejection::NotReady)
    );
}

#[test]
fn commands_t3_has_not_answered_are_bounded() {
    let mut follower = Follower::new("2".to_owned(), Vec::new());
    let _ = follower.frame(ServerFrame::Chunk {
        request_id: "2".to_owned(),
        values: vec![json!({"kind": "snapshot", "snapshotSequence": 1,
            "projection": {"thread": {"id": "thread-1"},
                "runs": [{"id": "run-1", "ordinal": 1, "status": "running"}]}})],
    });
    for n in 0..MAX_ORDERS {
        let order = order(Command::Cancel, n as u128);
        assert!(follower.dispatch(&order).is_ok());
        follower.sent(n.to_string(), &order);
    }
    assert_eq!(
        follower.dispatch(&order(Command::Cancel, 99)),
        Err(Rejection::QueueFull)
    );
}

#[test]
fn server_text_cannot_reach_the_terminal_as_escapes() {
    assert_eq!(plain("busy\u{1b}[2J\n"), "busy\u{fffd}[2J\u{fffd}");
}
