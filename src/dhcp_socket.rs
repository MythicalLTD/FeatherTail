//! DHCP server socket that knows which interface a request arrived on.
//!
//! One wildcard socket (0.0.0.0:67) with IP_PKTINFO, like dnsmasq: every request carries the
//! ingress interface index, and the reply is sent back out of that same interface. That is what
//! makes broadcast replies reach clients on a second bridge (e.g. vmbr1) instead of leaving via
//! the default-route interface, without one socket per bridge (which would make Linux deliver
//! each broadcast to every socket and produce duplicate answers).

use std::io;
use std::net::SocketAddr;

use tokio::net::UdpSocket;

pub struct DhcpSocket {
    inner: UdpSocket,
}

pub struct Received {
    pub len: usize,
    pub src: SocketAddr,
    pub ifindex: Option<u32>,
}

impl DhcpSocket {
    pub fn bind(bind: SocketAddr) -> io::Result<Self> {
        let std_socket = std::net::UdpSocket::bind(bind)?;
        std_socket.set_broadcast(true)?;
        std_socket.set_nonblocking(true)?;
        let inner = UdpSocket::from_std(std_socket)?;
        #[cfg(target_os = "linux")]
        linux::enable_pktinfo(&inner)?;
        Ok(Self { inner })
    }

    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<Received> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            let fd = self.inner.as_raw_fd();
            self.inner
                .async_io(tokio::io::Interest::READABLE, || linux::recv(fd, buf))
                .await
        }
        #[cfg(not(target_os = "linux"))]
        {
            let (len, src) = self.inner.recv_from(buf).await?;
            Ok(Received { len, src, ifindex: None })
        }
    }

    /// Sends `data` to `target`, out of interface `ifindex` when given.
    pub async fn send(&self, data: &[u8], target: SocketAddr, ifindex: Option<u32>) -> io::Result<usize> {
        #[cfg(target_os = "linux")]
        if let (Some(idx), SocketAddr::V4(v4)) = (ifindex, target) {
            use std::os::fd::AsRawFd;
            let fd = self.inner.as_raw_fd();
            return self
                .inner
                .async_io(tokio::io::Interest::WRITABLE, || linux::send(fd, data, v4, idx))
                .await;
        }
        let _ = ifindex;
        self.inner.send_to(data, target).await
    }
}

/// Interface name for an index (e.g. 5 -> "vmbr1").
pub fn interface_name(ifindex: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        linux::if_name(ifindex)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = ifindex;
        None
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io;
    use std::mem::{size_of, zeroed};
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    use std::os::fd::{AsRawFd, RawFd};

    use super::Received;

    pub fn enable_pktinfo(sock: &tokio::net::UdpSocket) -> io::Result<()> {
        let one: libc::c_int = 1;
        // SAFETY: valid fd, pointer/len describe a live c_int.
        let rc = unsafe {
            libc::setsockopt(
                sock.as_raw_fd(),
                libc::IPPROTO_IP,
                libc::IP_PKTINFO,
                &one as *const libc::c_int as *const libc::c_void,
                size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn recv(fd: RawFd, buf: &mut [u8]) -> io::Result<Received> {
        // SAFETY: all structures are zero-initialised POD and outlive the recvmsg call;
        // the control buffer is u64-aligned as cmsg requires.
        unsafe {
            let mut src: libc::sockaddr_in = zeroed();
            let mut iov = libc::iovec {
                iov_base: buf.as_mut_ptr() as *mut libc::c_void,
                iov_len: buf.len(),
            };
            let mut control = [0u64; 16];
            let mut msg: libc::msghdr = zeroed();
            msg.msg_name = &mut src as *mut libc::sockaddr_in as *mut libc::c_void;
            msg.msg_namelen = size_of::<libc::sockaddr_in>() as libc::socklen_t;
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = control.as_mut_ptr() as *mut libc::c_void;
            msg.msg_controllen = size_of::<[u64; 16]>() as _;

            let n = libc::recvmsg(fd, &mut msg, 0);
            if n < 0 {
                return Err(io::Error::last_os_error());
            }

            let mut ifindex = None;
            let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
            while !cmsg.is_null() {
                if (*cmsg).cmsg_level == libc::IPPROTO_IP && (*cmsg).cmsg_type == libc::IP_PKTINFO {
                    let info = std::ptr::read_unaligned(libc::CMSG_DATA(cmsg) as *const libc::in_pktinfo);
                    if info.ipi_ifindex > 0 {
                        ifindex = Some(info.ipi_ifindex as u32);
                    }
                }
                cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
            }

            let ip = Ipv4Addr::from(u32::from_be(src.sin_addr.s_addr));
            let port = u16::from_be(src.sin_port);
            Ok(Received {
                len: n as usize,
                src: SocketAddr::V4(SocketAddrV4::new(ip, port)),
                ifindex,
            })
        }
    }

    pub fn send(fd: RawFd, data: &[u8], target: SocketAddrV4, ifindex: u32) -> io::Result<usize> {
        // SAFETY: see recv; the pktinfo cmsg is written inside the aligned control buffer
        // sized with CMSG_SPACE.
        unsafe {
            let mut dst: libc::sockaddr_in = zeroed();
            dst.sin_family = libc::AF_INET as libc::sa_family_t;
            dst.sin_port = target.port().to_be();
            dst.sin_addr.s_addr = u32::from(*target.ip()).to_be();

            let mut iov = libc::iovec {
                iov_base: data.as_ptr() as *mut libc::c_void,
                iov_len: data.len(),
            };
            let mut control = [0u64; 8];
            let space = libc::CMSG_SPACE(size_of::<libc::in_pktinfo>() as u32) as usize;

            let mut msg: libc::msghdr = zeroed();
            msg.msg_name = &mut dst as *mut libc::sockaddr_in as *mut libc::c_void;
            msg.msg_namelen = size_of::<libc::sockaddr_in>() as libc::socklen_t;
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = control.as_mut_ptr() as *mut libc::c_void;
            msg.msg_controllen = space as _;

            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            (*cmsg).cmsg_level = libc::IPPROTO_IP;
            (*cmsg).cmsg_type = libc::IP_PKTINFO;
            (*cmsg).cmsg_len = libc::CMSG_LEN(size_of::<libc::in_pktinfo>() as u32) as _;
            let info = libc::in_pktinfo {
                ipi_ifindex: ifindex as libc::c_int,
                ipi_spec_dst: libc::in_addr { s_addr: 0 },
                ipi_addr: libc::in_addr { s_addr: 0 },
            };
            std::ptr::write_unaligned(libc::CMSG_DATA(cmsg) as *mut libc::in_pktinfo, info);

            let n = libc::sendmsg(fd, &msg, 0);
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(n as usize)
        }
    }

    pub fn if_name(ifindex: u32) -> Option<String> {
        let mut buf = [0 as libc::c_char; libc::IF_NAMESIZE];
        // SAFETY: buf is IF_NAMESIZE bytes as required by if_indextoname.
        let ptr = unsafe { libc::if_indextoname(ifindex, buf.as_mut_ptr()) };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: on success buf holds a NUL-terminated name.
        let name = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
        Some(name.to_string_lossy().into_owned())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pktinfo_roundtrip_on_loopback() {
        let server = DhcpSocket::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = server.inner.local_addr().unwrap();
        let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client.send_to(b"hello", addr).await.unwrap();

        let mut buf = [0u8; 64];
        let got = server.recv(&mut buf).await.unwrap();
        assert_eq!(&buf[..got.len], b"hello");
        let idx = got.ifindex.expect("pktinfo ifindex");
        assert_eq!(interface_name(idx).as_deref(), Some("lo"));

        // Reply forced out of the ingress interface.
        server.send(b"world", got.src, Some(idx)).await.unwrap();
        let (n, _) = client.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"world");
    }
}
