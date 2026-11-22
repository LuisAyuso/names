FROM archlinux:latest
RUN pacman -Sy --noconfirm iproute2 iptables-nft && pacman -Scc --noconfirm
COPY peer.sh /peer.sh
ENTRYPOINT ["sh", "/peer.sh"]
