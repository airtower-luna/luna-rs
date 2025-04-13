use crate::{set_rt_prio, PacketData, ReceivedPacket, ECHO_FLAG};

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


fn echo_log(
	sock: i32, max_len: usize, server: SocketAddr,
	logger: Option<mpsc::Sender<ReceivedPacket>>)
	-> Result<usize, Error>
{
	let flags = socket::MsgFlags::empty();
	let mut buffer = vec![0u8; max_len];
	let mut cmsgspace = cmsg_space!(TimeSpec);
	let mut iov = [IoSliceMut::new(&mut buffer)];
	let server_addr = SockaddrStorage::from(server);
	let mut count: usize = 0;

	if logger.is_none() {
		println!("{}", ReceivedPacket::header());
	}

	loop {
		let r = socket::recvmsg::<socket::SockaddrStorage>(
			sock, &mut iov, Some(&mut cmsgspace), flags)?;
		if r.bytes == 0 {
			// We get a zero bytes packet when the socket has been
			// shut down for reading.
			break;
		}
		if let Ok(recv) = ReceivedPacket::try_from(r) {
			if recv.source != server_addr {
				// wrong source
				continue;
			}
			if let Some(sender) = &logger {
				if let Err(_) = sender.send(recv) {
					// receiver hung up, no point in listening
					break;
				}
			} else {
				println!("{recv}");
			}
			count += 1;
		}
	}
	Ok(count)
}


/// Run the LUNA client in the current thread. Parameters are:
///
/// * server: address of the server to connect to
///
/// * buffer_size: size of send buffer, and receive buffer if `echo`
///   is true. If larger packets are requested, they will be truncated
///   to the buffer size.
///
/// * echo: if `true`, request that the server echo packets back to
///   the client
///
/// * receiver: read what packets to send from this channel
///
/// * echo_wait: if `Some`, the duration to wait for pending echo
///   packets after `receiver` has been closed
///
/// * echo_logger: if `Some`, information on received echoes (if
///   `echo` is `true` will be sent to this channel, otherwise it will
///   be written to standard output.
pub fn run(
	server: SocketAddr, buffer_size: usize, echo: bool,
	receiver: mpsc::Receiver<PacketData>,
	echo_wait: Option<Duration>,
	echo_logger: Option<mpsc::Sender<ReceivedPacket>>)
	-> Result<(), Box<dyn std::error::Error>>
{
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

	let et = if echo {
		let s = sock.as_raw_fd();
		Some(thread::spawn(move || echo_log(s, buffer_size, server, echo_logger)))
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

	let mut t = None;
	let mut seq: u32 = 0;

	let rusage_pre = resource::getrusage(resource::UsageWho::RUSAGE_THREAD)?;

	'send: loop {
		let next = match receiver.recv() {
			Ok(next) => next,
			Err(mpsc::RecvError) => {break 'send;}
		};
		t = t.or_else(|| Some(clock_gettime(CLOCK).unwrap()))
			.map(|u| u + next.delay);

		loop {
			match clock_nanosleep(
				CLOCK, ClockNanosleepFlags::TIMER_ABSTIME, t.as_ref().unwrap())
			{
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

		// prepare next packet
		seq += 1;
		buffer.splice(0..4, seq.to_be_bytes());
	}

	let rusage_post = resource::getrusage(resource::UsageWho::RUSAGE_THREAD)?;

	socket::shutdown(sock.as_raw_fd(), socket::Shutdown::Write)?;
	// delay so pending echos can arrive
	if let Some(w) = echo_wait {
		thread::sleep(w);
	}
	socket::shutdown(sock.as_raw_fd(), socket::Shutdown::Read)?;
	if let Some(t) = et {
		match t.join() {
			Err(e) => eprintln!("panic in echo thread: {e:?}"),
			Ok(r) => match r {
				Err(e) => eprintln!("error in echo thread: {e:?}"),
				Ok(count) => eprintln!("received {count} echo packets"),
			}
		};
	}

	eprintln!(
		"major page faults: {}, minor page faults: {}",
		rusage_post.major_page_faults() - rusage_pre.major_page_faults(),
		rusage_post.minor_page_faults() - rusage_pre.minor_page_faults()
	);
	Result::Ok(())
}
