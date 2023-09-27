#include <fcntl.h>     /* O_RDWR */
#include <stdio.h>     /* perror(), printf(), fprintf() */
#include <stdlib.h>    /* exit(), malloc(), free() */
#include <string.h>    /* memset(), memcpy() */
#include <sys/ioctl.h> /* ioctl() */
#include <unistd.h>    /* read(), close() */

/* includes for struct ifreq, etc */
#include <linux/if.h>
#include <linux/if_tun.h>
#include <sys/socket.h>
#include <sys/types.h>

int tun_open()
{
    struct ifreq ifr;
    int fd, err;

    if ((fd = open("/dev/net/tun", O_RDWR)) == -1) {
        perror("open /dev/net/tun");
        exit(1);
    }
    memset(&ifr, 0, sizeof(ifr));
    ifr.ifr_flags = IFF_TUN | IFF_NO_PI;
    strncpy(ifr.ifr_name, "tun1", IFNAMSIZ); // devname = "tun0" or "tun1", etc

    /* ioctl will use ifr.if_name as the name of TUN
     * interface to open: "tun0", etc. */
    if ((err = ioctl(fd, TUNSETIFF, (void*)&ifr)) < 0) {
        perror("ioctl TUNSETIFF");
        close(fd);
        exit(1);
    }

    /* After the ioctl call the fd is "connected" to tun device specified
     * by devname ("tun0", "tun1", etc)*/

    return fd;
}

int bind_eth0(int sockfd)
{
    const struct ifreq ifr = {
        .ifr_name = "enp7s0",
    };

    if (setsockopt(sockfd, SOL_SOCKET, SO_BINDTODEVICE, &ifr, sizeof(ifr)) < 0) {
        perror("setsockopt");
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
