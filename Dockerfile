FROM alpine:latest
ARG TARGETARCH
ARG TARGETVARIANT
RUN apk --no-cache add ca-certificates tini fuse3
RUN apk add tzdata && \
	cp /usr/share/zoneinfo/Asia/Shanghai /etc/localtime && \
	echo "Asia/Shanghai" > /etc/timezone && \
	apk del tzdata

RUN mkdir -p /etc/aliyundrive-fuse /mnt/aliyundrive
WORKDIR /root/
ADD aliyundrive-fuse-$TARGETARCH$TARGETVARIANT /usr/bin/aliyundrive-fuse

ENTRYPOINT ["/sbin/tini", "--"]
CMD ["/usr/bin/aliyundrive-fuse", "--workdir", "/etc/aliyundrive-fuse", "/mnt/aliyundrive"]
