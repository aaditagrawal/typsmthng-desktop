# COPR SCM builds run this target with network access in the SRPM buildroot.
.PHONY: srpm
srpm:
	dnf -y install git cargo rust rpm-build curl tar gzip xz uv
	bash packaging/copr/build-srpm.sh "$(outdir)"
