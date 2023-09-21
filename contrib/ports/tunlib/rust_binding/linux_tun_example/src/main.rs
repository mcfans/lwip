use std::{fs::File, os::fd::{FromRawFd, AsRawFd}, net::{Ipv4Addr, SocketAddr, SocketAddrV4}, io::{Read, Write}};
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use tun::tun::TunNetif;

extern "C" {
    fn tun_open() -> i32;
    fn bind_eth0(socket: i32) -> i32;
}

struct TcpHandler {
    handle: tokio::runtime::Handle
}

impl tun::tun::Pipe for TcpHandler {
    fn handle_new_connection(&self, conn: tun::tcp::TcpConnection, dst: SocketAddr) {
        println!("New Connection {:?}", dst);
        self.handle.spawn(async move {
            let mut tun_conn = conn;

            let socket = tokio::net::TcpSocket::new_v4().unwrap();
            let fd = socket.as_raw_fd();
            let res = unsafe { bind_eth0(fd) };
            if res != 0 {
                println!("Bind failed with error {}", res);
            }
            let outbound_conn = socket.connect(dst).await;

            if let Err(e) = outbound_conn {
                println!("Error connecting to {}: {:?}", dst, e);
                return;
            }
            let mut outbound_conn = outbound_conn.unwrap();
            println!("Connected to {}", dst);

            let res = tokio::io::copy_bidirectional(&mut tun_conn, &mut outbound_conn).await;
            println!("Connection to {} closed: {:?}", dst, res)
        });
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        // let socket = tokio::net::TcpSocket::new_v4().unwrap();
        // let fd = socket.as_raw_fd();
        // // let res = unsafe { bind_eth0(fd) };
        // let res = 0;
        // if res != 0 {
        //     println!("Bind failed with error {}", res);
        // }
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(45, 79, 112, 203), 4242));
        // let mut outbound_conn = socket.connect().await.unwrap();

        let mut outbound_conn = tokio::net::TcpStream::connect(addr).await.unwrap();

        let size = outbound_conn.write(b"Hello world").await.unwrap();

        println!("Writed {size}");

        let mut buf = [0u8; 11];
        
        outbound_conn.read(&mut buf).await.unwrap();

        println!("Read {}", std::str::from_utf8(&buf).unwrap());
    });

    let fd = unsafe { tun_open() };
    let mut file = unsafe { 
        File::from_raw_fd(fd)
    };
    let ip = Ipv4Addr::new(192, 18, 0, 1);
    let netmask = Ipv4Addr::new(255, 255, 255, 0);
    let gateway = Ipv4Addr::new(192, 18, 0, 1);
    let handler = TcpHandler { handle: runtime.handle().clone() };

    let mut tun = TunNetif::new(ip, netmask, gateway, Box::new(handler));

    println!("Opened tun device with fd {}", fd);

    tun.set_output_fn(Box::new(move |data| {
        // println!("Writing {} bytes to tun", data.len());
        let mut file = unsafe { File::from_raw_fd(fd) };
        let res = file.write_all(data);
        if let Err(e) = res {
            println!("Error writing to tun: {:?}", e);
        }
        std::mem::forget(file)
    }));

    loop {
        let mut buf = [0u8; 1504];
        let len = file.read(&mut buf).unwrap();
        // println!("Read {} bytes from tun", len);
        tun.input_data(&buf[0..len]);
    }
}
