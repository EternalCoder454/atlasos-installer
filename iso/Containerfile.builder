# Tools iso/build-iso.sh runs with, so the host needs none of them.
FROM registry.fedoraproject.org/fedora:44
RUN dnf install -y --setopt=install_weak_deps=False \
        squashfs-tools xorriso dosfstools mtools skopeo isomd5sum \
    && dnf clean all
