use luna_rs::{client, generator::Generator, server};
use clap::{Parser, Subcommand};
use nix::sys::{signal, socket::SockaddrStorage};
use std::{
	net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6, ToSocketAddrs}, sync::OnceLock, time::Duration
};
#[cfg(feature = "python")]
use std::{ffi::CString, fs, path::PathBuf};


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
	/// size of the send or receive buffer, larger packets cannot be
	/// sent, larger incoming packets will be truncated
	#[arg(short, long, default_value_t = 1500)]
	buffer_size: usize,
	#[command(subcommand)]
	command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
	Client {
		/// server to send to
		#[arg(short, long, default_value = "localhost:7800")]
		server: String,
		/// request packet echo from server
		#[arg(short, long, default_value_t = false)]
		echo: bool,
		/// generator selection
		#[arg(short, long, value_enum, default_value = "default")]
		generator: Generator,
		#[cfg(feature = "python")]
		#[arg(long, value_name = "MODULE_PY")]
		py_generator: Option<PathBuf>,
	},
	Server {
		/// port to listen on
		#[arg(short, long, default_value_t = 7800)]
		port: u16,
		/// local address to bind to for listening
		#[arg(short, long, default_value = "::")]
		bind: IpAddr,
	},
}


static SERVER_CLOSE: OnceLock<server::CloseHandle> = OnceLock::new();


extern fn handle_shutdown_sig(signal: libc::c_int) {
	let signal = signal::Signal::try_from(signal).unwrap();
	match signal {
		signal::Signal::SIGINT | signal::Signal::SIGTERM => SERVER_CLOSE.get().map(|h| h.close()),
		_ => panic!("signal handler was installed for unsupported signal"),
	};
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
	let args = Args::parse();
	#[cfg(debug_assertions)]
	eprintln!("{args:?}");
	match args.command {
		Commands::Client {
			server,
			echo,
			generator,
			#[cfg(feature = "python")]
			py_generator
		} => {
			#[cfg(feature = "python")]
			let generator = py_generator
				.map(|p| fs::read_to_string(p).unwrap())
				.map(|s| CString::new(s).unwrap())
				.map(|s| Generator::Py(s))
				.or(Some(generator));
			#[cfg(not(feature = "python"))]
			let generator = Some(generator);
			let receiver = generator.unwrap().run();
			let server_addr: SocketAddr = server
				.to_socket_addrs()
				.expect("cannot parse server address")
				.next().expect("no address");
			client::run(
				server_addr, args.buffer_size, echo, receiver,
				Some(Duration::from_millis(200)), None)?;
		},
		Commands::Server { port, bind } => {
			let bind_addr: SockaddrStorage = if bind.is_ipv6() {
				let s = format!("[{}]:{}", bind, port);
				SockaddrStorage::from(s.parse::<SocketAddrV6>()?)
			} else {
				let s = format!("{}:{}", bind, port);
				SockaddrStorage::from(s.parse::<SocketAddrV4>()?)
			};
			let mut srv = server::Server::new(bind_addr, args.buffer_size, None);
			let handle = srv.bind()?;
			if let Err(_) = SERVER_CLOSE.set(handle) {
				panic!("programming error: server close handle already set")
			}
			let handler = signal::SigHandler::Handler(handle_shutdown_sig);
			unsafe {
				signal::signal(signal::Signal::SIGINT, handler)?;
				signal::signal(signal::Signal::SIGTERM, handler)?;
			}
			srv.run()?;
		},
	}
	Result::Ok(())
}
