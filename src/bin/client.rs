use clap::Parser;
use luna_rs;
use nix::sys::socket::SockaddrStorage;

use std::io::{Error, IoSlice, IoSliceMut};
use std::net::{SocketAddr, ToSocketAddrs};
use std::os::fd::AsRawFd;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use nix::{cmsg_space, sys::{mman, resource, socket, time::TimeSpec}};
use nix::time::{ClockId, ClockNanosleepFlags, clock_gettime, clock_nanosleep};

static CLOCK: ClockId = ClockId::CLOCK_REALTIME;


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// server to send to
    #[arg(short, long, default_value = "localhost:7800")]
    pub server: String,
    /// request packet echo from server
    #[arg(short, long)]
    pub echo: bool,
}


fn generator(target: mpsc::Sender<TimeSpec>) {
    let step = TimeSpec::new(0, 500000000);
    for _ in 0..10 {
	target.send(step).unwrap();
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
	    sock.as_raw_fd(), &mut iov, Some(&mut cmsgspace), flags)?;
	let data = r.iovs().next().unwrap();

	if let Some(socket::ControlMessageOwned::ScmTimestampns(rtime)) = r.cmsgs()?.next() {
	    let addr = r.address.as_ref().unwrap();
	    if addr != &server_addr {
		// wrong source
		continue;
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
    let args = Args::parse();
    let echo = args.echo;
    let server: SocketAddr = args.server
	.to_socket_addrs()
	.expect("cannot parse server address")
	.next().expect("no address");

    if let Err(err) = luna_rs::set_rt_prio(20) {
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
    let mut buffer = vec![0u8; luna_rs::MIN_SIZE];
    if echo {
	buffer[20] = luna_rs::ECHO_FLAG;
    }

    let (sender, receiver) = mpsc::channel::<TimeSpec>();
    thread::spawn(move || generator(sender));
    if echo {
	let s = sock.as_raw_fd();
	thread::spawn(move || echo_log(s, luna_rs::MIN_SIZE, server));
    }

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
	socket::sendmsg(
	    sock.as_raw_fd(), &iov, &[], flags,
	    Option::<&SockaddrStorage>::None)?;
	#[cfg(debug_assertions)]
	eprintln!("sent {}: {:?}", seq, buffer);

	// prepare next packet
	seq += 1;
	buffer.splice(0..4, seq.to_be_bytes());
    }
    // wait a moment for pending echos
    thread::sleep(Duration::from_millis(500));
    let rusage_post = resource::getrusage(resource::UsageWho::RUSAGE_THREAD)?;
    eprintln!(
	"major page faults: {}, minor page faults: {}",
	rusage_post.major_page_faults() - rusage_pre.major_page_faults(),
	rusage_post.minor_page_faults() - rusage_pre.minor_page_faults()
    );
    Result::Ok(())
}
