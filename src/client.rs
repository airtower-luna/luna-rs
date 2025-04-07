use crate::{parse_int, set_rt_prio, PacketData, ECHO_FLAG, MIN_SIZE};

use nix::sys::socket::SockaddrStorage;

use std::io::{Error, IoSlice, IoSliceMut};
use std::net::SocketAddr;
use std::os::fd::AsRawFd;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use nix::{cmsg_space, sys::{mman, resource, socket, time::TimeSpec}};
use nix::time::{ClockId, ClockNanosleepFlags, clock_gettime, clock_nanosleep};

static CLOCK: ClockId = ClockId::CLOCK_REALTIME;


fn generator(target: mpsc::Sender<PacketData>) {
	let step = TimeSpec::new(0, 500000000);
	for _ in 0..10 {
		target.send(PacketData { delay: step, size: MIN_SIZE * 2 }).unwrap();
	}
}


fn echo_log(sock: i32, max_len: usize, server: SocketAddr) -> Result<(), Error> {
	let flags = socket::MsgFlags::empty();
	let mut buffer = vec![0u8; max_len];
	let mut cmsgspace = cmsg_space!(TimeSpec);
	let mut iov = [IoSliceMut::new(&mut buffer)];
	let server_addr = SockaddrStorage::from(server);

	println!("ktime\ttimestamp\tsequence\tsize");
	loop {
		let r = socket::recvmsg::<socket::SockaddrStorage>(
			sock, &mut iov, Some(&mut cmsgspace), flags)?;
		if r.bytes == 0 {
			// We get a zero bytes packet when the socket has been
			// shut down for reading.
			break;
		}
		let data = r.iovs().next().unwrap();

		if let Some(socket::ControlMessageOwned::ScmTimestampns(rtime)) = r.cmsgs()?.next() {
			let addr = r.address.as_ref().unwrap();
			if addr != &server_addr {
				// wrong source
				continue;
			}
			if r.bytes < MIN_SIZE {
				eprintln!("received packet is too short");
				continue;
			}
			let (seq, rest) = parse_int!(data, i32);
			let (sec, rest) = parse_int!(rest, i64);
			let (nsec, _) = parse_int!(rest, i64);
			let stamp = TimeSpec::new(sec, nsec);
			println!("{}.{:09}\t{}.{:09}\t{}\t{}", rtime.tv_sec(), rtime.tv_nsec(), stamp.tv_sec(), stamp.tv_nsec(), seq, r.bytes);
		}
	}
	Ok(())
}


pub fn run(server: SocketAddr, buffer_size: usize, echo: bool) -> Result<(), Box<dyn std::error::Error>> {
	if let Err(err) = set_rt_prio(20) {
		eprintln!("could not set realtime priority: {}", err);
	}

	let sock = socket::socket(
		if server.is_ipv6() {
			socket::AddressFamily::Inet6
		} else {
			socket::AddressFamily::Inet
		},
		socket::SockType::Datagram,
		socket::SockFlag::empty(),
		None
	)?;
	socket::setsockopt(&sock, socket::sockopt::ReceiveTimestampns, &true)?;
	socket::connect(sock.as_raw_fd(), &SockaddrStorage::from(server))?;

	let flags = socket::MsgFlags::empty();
	let mut buffer = vec![0u8; buffer_size];
	if echo {
		buffer[20] = ECHO_FLAG;
	}

	let (sender, receiver) = mpsc::channel::<PacketData>();
	thread::spawn(move || generator(sender));
	let et = if echo {
		let s = sock.as_raw_fd();
		Some(thread::spawn(move || echo_log(s, buffer_size, server)))
	} else {
		None
	};

	// Prevent swapping, if possible. Needs to be done after starting
	// threads because otherwise it'll fail if there's not enough
	// memory to do so without going over the limit of what can be
	// locked without CAP_IPC_LOCK.
	if let Err(e) = mman::mlockall(
		mman::MlockAllFlags::MCL_CURRENT
			| mman::MlockAllFlags::MCL_FUTURE) {
		eprintln!("could not lock memory: {}", e);
	}

	let mut t = clock_gettime(CLOCK)?;
	let mut seq: u32 = 0;

	let rusage_pre = resource::getrusage(resource::UsageWho::RUSAGE_THREAD)?;

	'send: loop {
		let next = match receiver.recv() {
			Ok(next) => next,
			Err(mpsc::RecvError) => {break 'send;}
		};
		t = t + next.delay;

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

		let iov = [IoSlice::new(&buffer[..buffer_size.min(next.size)])];
		socket::sendmsg(
			sock.as_raw_fd(), &iov, &[], flags,
			Option::<&SockaddrStorage>::None)?;
		#[cfg(debug_assertions)]
		eprintln!("sent {}: {:?}", seq, buffer);

		// prepare next packet
		seq += 1;
		buffer.splice(0..4, seq.to_be_bytes());
	}

	socket::shutdown(sock.as_raw_fd(), socket::Shutdown::Write)?;
	// wait a little longer for pending echos
	thread::sleep(Duration::from_millis(500));
	socket::shutdown(sock.as_raw_fd(), socket::Shutdown::Read)?;
	if let Some(t) = et {
		if let Err(e) = t.join() {
			eprintln!("{e:?}");
		};
	}

	let rusage_post = resource::getrusage(resource::UsageWho::RUSAGE_THREAD)?;
	eprintln!(
		"major page faults: {}, minor page faults: {}",
		rusage_post.major_page_faults() - rusage_pre.major_page_faults(),
		rusage_post.minor_page_faults() - rusage_pre.minor_page_faults()
	);
	Result::Ok(())
}
