//! Prints every instance in a Roblox model file with its stored properties.
//! Usage: cargo run --example rbxdump -- samples/R15Rig.rbxm [ClassFilter]
use rbx_dom_weak::WeakDom;
use rbx_types::Variant;

fn show(v: &Variant) -> String {
    match v {
        Variant::CFrame(c) => format!(
            "CFrame p=({:.3},{:.3},{:.3}) x=({:.2},{:.2},{:.2}) y=({:.2},{:.2},{:.2}) z=({:.2},{:.2},{:.2})",
            c.position.x, c.position.y, c.position.z,
            c.orientation.x.x, c.orientation.x.y, c.orientation.x.z,
            c.orientation.y.x, c.orientation.y.y, c.orientation.y.z,
            c.orientation.z.x, c.orientation.z.y, c.orientation.z.z,
        ),
        Variant::SharedString(_) | Variant::BinaryString(_) => "<binary>".into(),
        other => {
            let mut s = format!("{other:?}");
            s.truncate(90);
            s
        }
    }
}

fn walk(dom: &WeakDom, r: rbx_types::Ref, depth: usize, filter: Option<&str>) {
    let inst = dom.get_by_ref(r).unwrap();
    let pad = "  ".repeat(depth);
    let hit = filter.is_none_or(|f| inst.class.as_str() == f);
    if hit {
        println!("{pad}{} \"{}\" [{}]", inst.class, inst.name, inst.referent());
        if filter.is_some() {
            let mut keys: Vec<_> = inst.properties.iter().collect();
            keys.sort_by_key(|(k, _)| k.as_str());
            for (k, v) in keys {
                println!("{pad}    {k} = {}", show(v));
            }
        }
    }
    for c in inst.children() {
        walk(dom, *c, depth + 1, filter);
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("path");
    let filter = args.next();
    let bytes = std::fs::read(&path).unwrap();
    let dom = rbx_binary::from_reader(&bytes[..]).unwrap();
    for c in dom.root().children() {
        walk(&dom, *c, 0, filter.as_deref());
    }
}
