//! Confirm whether FieldMask path "message.agent_output.text" on api::Message
//! descriptor works. If `get_field_by_name("message")` returns None on the
//! outer `Message` proto (because `message` is the oneof name, not a field),
//! the whole FieldMask append silently no-ops — which would explain why Warp
//! renders blank.

use prost_reflect::ReflectMessage;
use warp_multi_agent_api as wmaa;

fn main() {
    let desc = wmaa::MESSAGE_DESCRIPTOR.clone();
    println!("descriptor: {}", desc.full_name());
    println!("--- fields ---");
    for f in desc.fields() {
        println!(
            "  #{:<3} name={:?} full_name={:?} kind={:?}",
            f.number(),
            f.name(),
            f.full_name(),
            f.kind()
        );
    }
    println!("--- oneofs ---");
    for o in desc.oneofs() {
        println!(
            "  oneof name={:?} fields=[{}]",
            o.name(),
            o.fields()
                .map(|f| f.name().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!();

    println!("--- get_field_by_name tests ---");
    println!(
        "  'message' -> {:?}",
        desc.get_field_by_name("message").is_some()
    );
    println!(
        "  'agent_output' -> {:?}",
        desc.get_field_by_name("agent_output").is_some()
    );
    println!("  'text' -> {:?}", desc.get_field_by_name("text").is_some());

    println!();
    println!("--- simulate the FieldMask apply_path walk ---");

    // Build a Message with AgentOutput, try to apply field mask append
    use prost_reflect::DynamicMessage;

    let base = wmaa::Message {
        id: "msg-1".to_string(),
        task_id: "task-1".to_string(),
        message: Some(wmaa::message::Message::AgentOutput(
            wmaa::message::AgentOutput {
                text: "".to_string(),
            },
        )),
        ..Default::default()
    };
    let patch = wmaa::Message {
        id: "msg-1".to_string(),
        task_id: "task-1".to_string(),
        message: Some(wmaa::message::Message::AgentOutput(
            wmaa::message::AgentOutput {
                text: "hello".to_string(),
            },
        )),
        ..Default::default()
    };

    let mask_variants = [
        vec!["message.agent_output.text".to_string()],
        vec!["agent_output.text".to_string()],
    ];

    for paths in &mask_variants {
        println!("\n>> mask paths = {:?}", paths);
        let mut dyn_target = DynamicMessage::new(desc.clone());
        dyn_target.transcode_from(&base).unwrap();
        let mut dyn_patch = DynamicMessage::new(desc.clone());
        dyn_patch.transcode_from(&patch).unwrap();

        for path in paths {
            walk(
                &mut dyn_target,
                &dyn_patch,
                &path.split('.').collect::<Vec<_>>(),
                0,
            );
        }

        let merged: wmaa::Message = dyn_target.transcode_to().unwrap();
        let text = match &merged.message {
            Some(wmaa::message::Message::AgentOutput(a)) => a.text.clone(),
            _ => "<no agent_output>".to_string(),
        };
        println!("  AFTER MERGE text = {:?}", text);
    }
}

fn walk(
    target: &mut prost_reflect::DynamicMessage,
    patch: &prost_reflect::DynamicMessage,
    segs: &[&str],
    depth: usize,
) {
    let indent = "  ".repeat(depth);
    let Some(name) = segs.first() else {
        println!("{}(end)", indent);
        return;
    };
    match target.descriptor().get_field_by_name(name) {
        Some(f) => {
            println!(
                "{}seg={:?} -> FOUND field #{} name={:?} kind={:?}",
                indent,
                name,
                f.number(),
                f.name(),
                f.kind()
            );
            if segs.len() == 1 {
                let pv = patch.get_field(&f).into_owned();
                println!("{}  setting target.{} = {:?}", indent, f.name(), pv);
                target.try_set_field(&f, pv).unwrap();
                return;
            }
            // recurse
            use prost_reflect::Value;
            let tv = target.get_field_mut(&f);
            let pv = patch.get_field(&f);
            match (&mut *tv, pv.as_ref()) {
                (Value::Message(t), Value::Message(p)) => walk(t, p, &segs[1..], depth + 1),
                _ => println!("{}  (not a message; cannot recurse)", indent),
            }
        }
        None => {
            println!(
                "{}seg={:?} -> NOT FOUND in descriptor {} (NO-OP — this is the bug)",
                indent,
                name,
                target.descriptor().full_name()
            );
        }
    }
}
