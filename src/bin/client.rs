use luna_rs;
use nix::sys::socket::SockaddrIn6;

use std::io::{Error, IoSlice, IoSliceMut};
use std::os::fd::AsRawFd;
use std::str::FromStr;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use nix::{cmsg_space, sys::{mman, socket, time::TimeSpec}};
use nix::time::{ClockId, ClockNanosleepFlags, clock_gettime, clock_nanosleep};

static CLOCK: ClockId = ClockId::CLOCK_REALTIME;


fn generator(target: mpsc::Sender<TimeSpec>) {
    let step = TimeSpec::new(0, 500000000);
    for _ in 0..10 {
	target.send(step).unwrap();
    }
}


fn echo_log(sock: i32, max_len: usize, server: Option<SockaddrIn6>) -> Result<(), Error> {
    let flags = socket::MsgFlags::empty();
    let mut buffer = vec![0u8; max_len];
    let mut cmsgspace = cmsg_space!(TimeSpec);
    let mut iov = [IoSliceMut::new(&mut buffer)];

    println!("ktime\ttimestamp\tsequence\tsize");
    loop {
	let r = socket::recvmsg::<socket::SockaddrIn6>(sock.as_raw_fd(), &mut iov, Some(&mut cmsgspace), flags)?;
	let data = r.iovs().next().unwrap();

	if let Some(socket::ControlMessageOwned::ScmTimestampns(rtime)) = r.cmsgs()?.next() {
	    if let Some(s) = server {
		let addr = r.address.as_ref().unwrap();
		if addr != &s {
		    // wrong source
		    continue;
		}
	    }
	    if r.bytes < luna_rs::MIN_SIZE {
		eprintln!("received packet is too short");
		continue;
	    }
	    let (b, rest) = data.split_at(size_of::<i32>());
	    let seq = i32::from_be_bytes(b.try_into().unwrap());
	    let (b, rest) = rest.split_at(size_of::<i64>());
	    let sec = i64::from_be_bytes(b.try_into().unwrap());
	    let (b, _) = rest.split_at(size_of::<i64>());
	    let nsec = i64::from_be_bytes(b.try_into().unwrap());
	    let stamp = TimeSpec::new(sec, nsec);
	    println!("{}.{:09}\t{}.{:09}\t{}\t{}", rtime.tv_sec(), rtime.tv_nsec(), stamp.tv_sec(), stamp.tv_nsec(), seq, r.bytes);
	}
    }
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    // TODO: should be configurable
    let echo = true;
    let server = socket::SockaddrIn6::from_str("[::1]:7800")?;

    // prevent swapping, if possible
    if let Err(e) = mman::mlockall(
	mman::MlockAllFlags::MCL_CURRENT
	    | mman::MlockAllFlags::MCL_FUTURE) {
	eprintln!("could not lock memory: {}", e);
    }

    if let Err(err) = luna_rs::set_rt_prio(20) {
	eprintln!("could not set realtime priority: {}", err);
    }

    let sock = socket::socket(
	socket::AddressFamily::Inet6,
	socket::SockType::Datagram,
	socket::SockFlag::empty(),
	None
    )?;
    socket::setsockopt(&sock, socket::sockopt::ReceiveTimestampns, &true)?;
    socket::connect(sock.as_raw_fd(), &server)?;

    let flags = socket::MsgFlags::empty();
    let mut buffer = vec![0u8; luna_rs::MIN_SIZE];
    if echo {
	buffer[20] = luna_rs::ECHO_FLAG;
    }

    let (sender, receiver) = mpsc::channel::<TimeSpec>();
    thread::spawn(move || generator(sender));
    let s = sock.as_raw_fd();
    thread::spawn(move || echo_log(s, luna_rs::MIN_SIZE, Some(server)));

    let mut t = clock_gettime(CLOCK)?;
    let mut seq: u32 = 0;

    'send: loop {
	t = match receiver.recv() {
	    Ok(next) => t + next,
	    Err(mpsc::RecvError) => {break 'send;}
	};
	loop {
	    match clock_nanosleep(CLOCK, ClockNanosleepFlags::TIMER_ABSTIME, &t) {
		Ok(_) => break,
		// restart sleep if it was interrupted
		Err(nix::Error::EINTR) => (),
		Err(e) => return Result::Err(Box::new(e))
	    }
	}

	// write current time to packet
	let current = clock_gettime(CLOCK)?;
	buffer.splice(4..12, current.tv_sec().to_be_bytes());
	buffer.splice(12..20, current.tv_nsec().to_be_bytes());

	let iov = [IoSlice::new(&buffer)];
	socket::sendmsg(sock.as_raw_fd(), &iov, &[], flags, Some(&server))?;
	#[cfg(debug_assertions)]
	eprintln!("sent {}: {:?}", seq, buffer);

	// prepare next packet
	seq += 1;
	buffer.splice(0..4, seq.to_be_bytes());
    }
    // wait a moment for pending echos
    thread::sleep(Duration::from_millis(500));
    Result::Ok(())
}
