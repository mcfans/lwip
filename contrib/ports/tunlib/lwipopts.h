#define NO_SYS                          1
#define SYS_LIGHTWEIGHT_PROT            1

#define LWIP_SOCKET                     0
#define LWIP_NETCONN                    0
#define MEM_ALIGNMENT                   8
#define MEM_SIZE                        10485760
#define MEMP_NUM_PBUF                   131072
#define MEMP_NUM_TCP_PCB                2048
#define MEMP_NUM_TCP_PCB_LISTEN         2048
// #define MEMP_OVERFLOW_CHECK             1
// #define MEM_SANITY_CHECK                1
// #define MEMP_SANITY_CHECK               1
#define LWIP_IPV4                       1

// #define TCP_DEBUG                  LWIP_DBG_ON
// #define TCP_INPUT_DEBUG            LWIP_DBG_ON
// #define TCP_OUTPUT_DEBUG           LWIP_DBG_ON
// #define TCP_RTO_DEBUG              LWIP_DBG_ON
// #define TCP_CWND_DEBUG             LWIP_DBG_ON
// #define TCP_WND_DEBUG              LWIP_DBG_ON
// #define TCP_FR_DEBUG               LWIP_DBG_ON
// #define TCP_QLEN_DEBUG             LWIP_DBG_ON
// #define TCP_RST_DEBUG              LWIP_DBG_ON

#define TCP_XX_DEBUG               LWIP_DBG_ON

#define LWIP_DBG_MIN_LEVEL              LWIP_DBG_LEVEL_ALL

#define TCP_MSS         1460
// We don't use this. This is here to slient tcp check.
#define TCP_SNDLOWAT    1024

#define TCP_RCV_SCALE   0xD
#define TCP_WND         16777216
#define LWIP_WND_SCALE  1
#define TCP_QUEUE_OOSEQ 1
#define TCP_SND_BUF     32768
#define LWIP_TCP_SACK_OUT 1

#define LWIP_TIMERS 1

#define PBUF_LINK_HLEN                  16

#define MEMP_NUM_TCP_SEG                TCP_SND_QUEUELEN
#define PBUF_POOL_SIZE                  16384
#define PBUF_POOL_BUFSIZE               LWIP_MEM_ALIGN_SIZE(TCP_MSS+40+PBUF_LINK_HLEN)
// #define IP_DEBUG                        LWIP_DBG_ON
// #define TCP_DEBUG                       LWIP_DBG_ON
// #define TCP_INPUT_DEBUG                 LWIP_DBG_ON
