//! 静态示例数据。
//!
//! M0 只做界面骨架，这里的数据不来自真实媒体库，也不触发任何音频或网络请求。

pub struct Album {
    pub title: &'static str,
    pub artist: &'static str,
    pub year: u16,
    pub tracks: u8,
    pub format: &'static str,
}

pub struct Track {
    pub no: u8,
    pub title: &'static str,
    pub artist: &'static str,
    pub album: &'static str,
    pub duration: u32,
    pub format: &'static str,
}

pub struct Playlist {
    pub name: &'static str,
    pub count: usize,
    pub note: &'static str,
}

pub struct Source {
    pub name: &'static str,
    pub detail: &'static str,
    pub status: &'static str,
    pub ok: bool,
    pub action: &'static str,
}

pub const ALBUMS: [Album; 8] = [
    Album {
        title: "夜航西飞",
        artist: "郭顶",
        year: 2016,
        tracks: 10,
        format: "FLAC",
    },
    Album {
        title: "Random Access Memories",
        artist: "Daft Punk",
        year: 2013,
        tracks: 13,
        format: "FLAC",
    },
    Album {
        title: "山丘",
        artist: "李宗盛",
        year: 2013,
        tracks: 11,
        format: "MP3",
    },
    Album {
        title: "In Rainbows",
        artist: "Radiohead",
        year: 2007,
        tracks: 10,
        format: "FLAC",
    },
    Album {
        title: "冀西南林路行",
        artist: "万能青年旅店",
        year: 2020,
        tracks: 9,
        format: "FLAC",
    },
    Album {
        title: "Blonde",
        artist: "Frank Ocean",
        year: 2016,
        tracks: 17,
        format: "AAC",
    },
    Album {
        title: "丑奴儿",
        artist: "老王乐队",
        year: 2017,
        tracks: 10,
        format: "MP3",
    },
    Album {
        title: "Currents",
        artist: "Tame Impala",
        year: 2015,
        tracks: 13,
        format: "FLAC",
    },
];

pub const TRACKS: [Track; 12] = [
    Track {
        no: 1,
        title: "凄美地",
        artist: "郭顶",
        album: "夜航西飞",
        duration: 272,
        format: "FLAC",
    },
    Track {
        no: 2,
        title: "水星记",
        artist: "郭顶",
        album: "夜航西飞",
        duration: 325,
        format: "FLAC",
    },
    Track {
        no: 3,
        title: "Get Lucky",
        artist: "Daft Punk",
        album: "Random Access Memories",
        duration: 369,
        format: "FLAC",
    },
    Track {
        no: 4,
        title: "Instant Crush",
        artist: "Daft Punk",
        album: "Random Access Memories",
        duration: 337,
        format: "FLAC",
    },
    Track {
        no: 5,
        title: "山丘",
        artist: "李宗盛",
        album: "山丘",
        duration: 384,
        format: "MP3",
    },
    Track {
        no: 6,
        title: "给自己的歌",
        artist: "李宗盛",
        album: "山丘",
        duration: 286,
        format: "MP3",
    },
    Track {
        no: 7,
        title: "Weird Fishes / Arpeggi",
        artist: "Radiohead",
        album: "In Rainbows",
        duration: 318,
        format: "FLAC",
    },
    Track {
        no: 8,
        title: "Nude",
        artist: "Radiohead",
        album: "In Rainbows",
        duration: 255,
        format: "FLAC",
    },
    Track {
        no: 9,
        title: "杀死那个石家庄人",
        artist: "万能青年旅店",
        album: "冀西南林路行",
        duration: 344,
        format: "FLAC",
    },
    Track {
        no: 10,
        title: "秦皇岛",
        artist: "万能青年旅店",
        album: "冀西南林路行",
        duration: 484,
        format: "FLAC",
    },
    Track {
        no: 11,
        title: "Pink + White",
        artist: "Frank Ocean",
        album: "Blonde",
        duration: 184,
        format: "AAC",
    },
    Track {
        no: 12,
        title: "The Less I Know the Better",
        artist: "Tame Impala",
        album: "Currents",
        duration: 216,
        format: "FLAC",
    },
];

pub const PLAYLISTS: [Playlist; 5] = [
    Playlist {
        name: "深夜驾车",
        count: 42,
        note: "过去两周常听",
    },
    Playlist {
        name: "工作专注",
        count: 128,
        note: "无人声为主",
    },
    Playlist {
        name: "中文独立",
        count: 76,
        note: "自动收录 2015 年后",
    },
    Playlist {
        name: "2026 新歌",
        count: 31,
        note: "每周更新",
    },
    Playlist {
        name: "全部收藏",
        count: 1204,
        note: "所有打过星标的曲目",
    },
];

pub const SOURCES: [Source; 3] = [
    Source {
        name: "本地媒体库",
        detail: "12,480 首 · 482 GB · 上次扫描 12 分钟前",
        status: "就绪",
        ok: true,
        action: "重新扫描",
    },
    Source {
        name: "Navidrome · studio",
        detail: "studio.example.net · OpenSubsonic 1.16.1",
        status: "在线",
        ok: true,
        action: "断开连接",
    },
    Source {
        name: "网络电台",
        detail: "尚未添加电台地址",
        status: "未配置",
        ok: false,
        action: "添加电台",
    },
];
