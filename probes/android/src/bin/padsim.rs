//! Creates a virtual gamepad through /dev/uinput (needs root), presses a few
//! buttons and moves the sticks and d-pad, then removes it. Used by
//! scripts/android/deploy.sh `pad` to test gamepad input on a device without
//! a physical controller.
#[cfg(target_os = "linux")]
fn main() {}

#[cfg(target_os = "android")]
fn main() {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::thread::sleep;
    use std::time::Duration;

    const EV_SYN: u16 = 0;
    const EV_KEY: u16 = 1;
    const EV_ABS: u16 = 3;
    const UI_SET_EVBIT: u64 = 0x4004_5564;
    const UI_SET_KEYBIT: u64 = 0x4004_5565;
    const UI_SET_ABSBIT: u64 = 0x4004_5567;
    const UI_DEV_CREATE: u64 = 0x5501;
    const UI_DEV_DESTROY: u64 = 0x5502;
    // BTN_A, B, X, Y, TL, TR, SELECT, START, MODE, THUMBL, THUMBR
    const KEYS: [u16; 11] = [
        0x130, 0x131, 0x133, 0x134, 0x136, 0x137, 0x13a, 0x13b, 0x13c, 0x13d, 0x13e,
    ];
    // ABS_X, Y, Z (LT), RX, RY, RZ (RT), HAT0X, HAT0Y
    const AXES: [(u16, i32, i32); 8] = [
        (0, -32768, 32767),
        (1, -32768, 32767),
        (2, 0, 255),
        (3, -32768, 32767),
        (4, -32768, 32767),
        (5, 0, 255),
        (0x10, -1, 1),
        (0x11, -1, 1),
    ];

    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .expect("open /dev/uinput");
    let fd = f.as_raw_fd();
    let io = |req: u64, v: i32| unsafe { libc::ioctl(fd, req as _, v) };
    io(UI_SET_EVBIT, EV_KEY as i32);
    io(UI_SET_EVBIT, EV_ABS as i32);
    for k in KEYS {
        io(UI_SET_KEYBIT, k as i32);
    }
    for (a, _, _) in AXES {
        io(UI_SET_ABSBIT, a as i32);
    }
    // struct uinput_user_dev: name[80], input_id{bustype,vendor,product,version},
    // ff_effects_max, absmax[64], absmin[64], absfuzz[64], absflat[64]
    let mut dev = vec![0u8; 80 + 8 + 4 + 4 * 64 * 4];
    dev[..19].copy_from_slice(b"baba-port padsim X3");
    for (i, v) in [0x03u16, 0x045e, 0x028e, 0x0114].iter().enumerate() {
        dev[80 + 2 * i..82 + 2 * i].copy_from_slice(&v.to_le_bytes());
    }
    for (a, min, max) in AXES {
        let a = a as usize;
        dev[92 + 4 * a..96 + 4 * a].copy_from_slice(&max.to_le_bytes());
        dev[92 + 256 + 4 * a..96 + 256 + 4 * a].copy_from_slice(&min.to_le_bytes());
    }
    f.write_all(&dev).expect("write uinput_user_dev");
    assert_eq!(io(UI_DEV_CREATE, 0), 0, "UI_DEV_CREATE");
    println!("padsim: device created");
    sleep(Duration::from_millis(1500));

    let mut send = |t: u16, c: u16, v: i32| {
        let mut ev = [0u8; 24]; // timeval(16) type code value
        ev[16..18].copy_from_slice(&t.to_le_bytes());
        ev[18..20].copy_from_slice(&c.to_le_bytes());
        ev[20..24].copy_from_slice(&v.to_le_bytes());
        f.write_all(&ev).unwrap();
        if t != EV_SYN {
            let mut syn = [0u8; 24];
            syn[16..18].copy_from_slice(&EV_SYN.to_le_bytes());
            f.write_all(&syn).unwrap();
        }
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        // Default: every button once, then sticks, d-pad and a trigger.
        for (name, k) in [
            ("A", 0x130),
            ("B", 0x131),
            ("X", 0x133),
            ("Y", 0x134),
            ("START", 0x13b),
        ] {
            println!("padsim: press {name}");
            send(EV_KEY, k, 1);
            sleep(Duration::from_millis(120));
            send(EV_KEY, k, 0);
            sleep(Duration::from_millis(250));
        }
        for (name, a, v) in [
            ("left stick right", 0, 32767),
            ("left stick centre", 0, 0),
            ("right stick up", 4, -32768),
            ("right stick centre", 4, 0),
            ("d-pad right", 0x10, 1),
            ("d-pad centre", 0x10, 0),
            ("right trigger", 5, 255),
            ("right trigger off", 5, 0),
        ] {
            println!("padsim: {name}");
            send(EV_ABS, a, v);
            sleep(Duration::from_millis(250));
        }
    }
    // Scripted: a b x y start select (buttons), up down left right (d-pad),
    // stick-right/left/up/down (left stick), NNNms (pause).
    for a in &args {
        let press = |send: &mut dyn FnMut(u16, u16, i32), t: u16, c: u16, on: i32, off: i32| {
            send(t, c, on);
            sleep(Duration::from_millis(120));
            send(t, c, off);
            sleep(Duration::from_millis(350));
        };
        println!("padsim: {a}");
        match a.as_str() {
            "a" => press(&mut send, EV_KEY, 0x130, 1, 0),
            "b" => press(&mut send, EV_KEY, 0x131, 1, 0),
            "x" => press(&mut send, EV_KEY, 0x133, 1, 0),
            "y" => press(&mut send, EV_KEY, 0x134, 1, 0),
            "start" => press(&mut send, EV_KEY, 0x13b, 1, 0),
            "select" => press(&mut send, EV_KEY, 0x13a, 1, 0),
            "right" => press(&mut send, EV_ABS, 0x10, 1, 0),
            "left" => press(&mut send, EV_ABS, 0x10, -1, 0),
            "down" => press(&mut send, EV_ABS, 0x11, 1, 0),
            "up" => press(&mut send, EV_ABS, 0x11, -1, 0),
            "stick-right" => press(&mut send, EV_ABS, 0, 32767, 0),
            "stick-left" => press(&mut send, EV_ABS, 0, -32768, 0),
            "stick-down" => press(&mut send, EV_ABS, 1, 32767, 0),
            "stick-up" => press(&mut send, EV_ABS, 1, -32768, 0),
            ms if ms.ends_with("ms") => {
                sleep(Duration::from_millis(ms.trim_end_matches("ms").parse().unwrap_or(0)))
            }
            other => eprintln!("padsim: unknown action {other}"),
        }
    }
    sleep(Duration::from_millis(300));
    io(UI_DEV_DESTROY, 0);
    println!("padsim: device removed");
}
