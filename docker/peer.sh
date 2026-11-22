# LOSS drops that share of packets, as Wi-Fi does; DROP_BROADCAST drops
# incoming broadcasts, as Android does without a multicast lock.
set -e
[ "$LOSS" != 0% ] && tc qdisc add dev eth0 root netem loss "$LOSS"
[ -n "$DROP_BROADCAST" ] && iptables -A INPUT -m pkttype --pkt-type broadcast -j DROP
exec /names --headless /tmp/names.txt "$NAME"
