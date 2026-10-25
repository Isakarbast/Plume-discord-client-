# Fedora / COPR package. COPR needs "enable network" for cargo, or vendor
# the crates first (cargo vendor) and add them as a second source.
Name:           plume
Version:        0.1.0
Release:        1%{?dist}
Summary:        A lightweight native Discord client
License:        GPL-3.0-or-later
URL:            https://github.com/Isakarbast/Plume-discord-client-
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  cargo >= 1.88
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  pkgconfig(gtk4) >= 4.14
BuildRequires:  pkgconfig(libadwaita-1) >= 1.6
BuildRequires:  pkgconfig(alsa)
BuildRequires:  cmake
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib
Requires:       gtk4
Requires:       libadwaita
Requires:       alsa-lib

%description
Plume is a Discord client written in Rust with GTK 4 and libadwaita, without
Electron. It includes a desktop app (plume) and a tiny terminal client
(plume-tui). Not affiliated with Discord; third-party clients are against
Discord's Terms of Service.

%prep
%autosetup -n Plume-discord-client--%{version}

%build
cargo build --release -p plume-gtk
cargo build --profile feather -p plume-tui

%install
install -Dm755 target/release/plume %{buildroot}%{_bindir}/plume
install -Dm755 target/feather/plume-tui %{buildroot}%{_bindir}/plume-tui
install -Dm644 packaging/dev.plume.Plume.desktop %{buildroot}%{_datadir}/applications/dev.plume.Plume.desktop
install -Dm644 packaging/dev.plume.Plume.metainfo.xml %{buildroot}%{_metainfodir}/dev.plume.Plume.metainfo.xml
install -Dm644 assets/icons/dev.plume.Plume.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/dev.plume.Plume.svg

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/dev.plume.Plume.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/dev.plume.Plume.metainfo.xml

%files
%license LICENSE
%doc README.md
%{_bindir}/plume
%{_bindir}/plume-tui
%{_datadir}/applications/dev.plume.Plume.desktop
%{_metainfodir}/dev.plume.Plume.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/dev.plume.Plume.svg

%changelog
* Sun Oct 25 2026 Plume contributors - 0.1.0-1
- First release
