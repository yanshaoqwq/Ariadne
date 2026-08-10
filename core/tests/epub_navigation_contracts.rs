//! U127：导出的 EPUB 必须含导航文档（`properties="nav"` 的 XHTML）。
//!
//! 缺陷：`render_chapters_epub` 声明 `version="3.0"`，但 manifest 里既没有
//! EPUB 3 的 nav 文档、也没有 EPUB 2 的 `toc.ncx`。后果是阅读器目录面板为空、
//! **无法跳章**，epubcheck 判不合规——而「跳到第 N 章」对长篇小说恰恰是最常用的操作。
//!
//! 审查报告已用真实产物逐项实测过：`mimetype` 首项且 stored、zip 完整、
//! 必需文件齐备、manifest href 全部有效——**除导航外全部合规**。所以本文件
//! 只针对导航这一条，同时把「原本已合规的部分不许被改坏」一并钉住。
//!
//! 判据一律走**真实解压 + 解析**，不做字节窗口扫描：
//! `windows(..).any(..)` 这类断言无法区分「文件真的存在」与「某处字符串恰好撞上」。

use std::io::{Cursor, Read};

use ariadne::documents::{
    ChapterDocumentEntry, ChapterDocumentIndex, ChapterDocumentKind, DocumentRepository,
    DocumentWriteRequest, FileDocumentService,
};
use ariadne::frontend::{
    export_chapters_combined, project_document_permission, ChapterExportFormat,
};

/// 建三章（含一个带 XML 元字符的标题）并导出 EPUB，返回 zip 字节。
///
/// 标题里刻意放 `&` 与 `<`：未转义时整个 nav 会变成非良构 XML，
/// 阅读器连目录都解析不出来——比没有目录更糟。
fn export_three_chapter_epub(temp: &std::path::Path) -> Vec<u8> {
    let service = FileDocumentService::new(
        project_document_permission(temp),
        temp.join(".runtime").join("artifacts"),
    );
    let titles = ["第一章 启程", "第二章 影 & 光 <序>", "第三章 归途"];
    let mut entries = Vec::new();
    for (index, title) in titles.iter().enumerate() {
        let path = temp.join("documents").join(format!("chapter{index}.md"));
        service
            .save_document(DocumentWriteRequest {
                path: path.clone(),
                content: format!("{title}的正文内容。\n\n第二段。"),
                format: None,
                base_version: None,
            })
            .unwrap();
        entries.push(ChapterDocumentEntry {
            chapter_id: format!("stage1:chapter{index}"),
            document_id: path.to_string_lossy().to_string(),
            path,
            title: (*title).to_owned(),
            order: index as u64 + 1,
            kind: ChapterDocumentKind::ChapterBody,
            version: "v1".to_owned(),
            word_count: Some(8),
            outline_ref: None,
        });
    }
    let index = ChapterDocumentIndex::new("v1", entries).unwrap();
    let selected = (0..titles.len())
        .map(|i| format!("stage1:chapter{i}"))
        .collect::<Vec<_>>();
    export_chapters_combined(
        &service,
        &index,
        &selected,
        "exports/book.epub",
        ChapterExportFormat::Epub,
    )
    .unwrap();
    std::fs::read(temp.join(".runtime/artifacts/exports/book.epub")).unwrap()
}

/// 从 zip 里读一个条目的文本内容。
fn read_entry(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.to_vec()))
        .unwrap_or_else(|error| panic!("EPUB 必须是可解析的 zip：{error}"));
    let mut file = archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("EPUB 缺少条目 {name}"));
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    text
}

fn entry_names(bytes: &[u8]) -> Vec<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_owned())
        .collect()
}

/// **U127 主用例**：manifest 必须声明一个 `properties="nav"` 的导航文档，且该文件真实存在。
#[test]
fn u127_epub_manifest_declares_nav_document_and_file_exists() {
    let temp = tempfile::tempdir().unwrap();
    let bytes = export_three_chapter_epub(temp.path());

    let opf = read_entry(&bytes, "OEBPS/content.opf");
    assert!(
        opf.contains(r#"properties="nav""#),
        "U127：EPUB 声明 version=3.0 却没有 properties=\"nav\" 的导航文档，\
         阅读器目录面板为空、无法跳章，epubcheck 判不合规。content.opf=\n{opf}"
    );
    assert!(
        entry_names(&bytes).iter().any(|name| name == "OEBPS/nav.xhtml"),
        "manifest 声明了 nav 但 zip 里没有这个文件——声明与产物不一致比没有导航更难排查"
    );
}

/// 导航项必须覆盖**全部**章节，且指向真实存在的章节文件。
///
/// 只声明一个 nav 却漏章，用户点开目录只看到第一章——比目录为空更容易被误判为「已修好」。
#[test]
fn u127_nav_lists_every_chapter_and_links_resolve() {
    let temp = tempfile::tempdir().unwrap();
    let bytes = export_three_chapter_epub(temp.path());
    let nav = read_entry(&bytes, "OEBPS/nav.xhtml");
    let names = entry_names(&bytes);

    for index in 0..3 {
        let href = format!("chapter{index}.xhtml");
        assert!(
            nav.contains(&format!(r#"href="{href}""#)),
            "导航缺少第 {index} 章的链接。nav=\n{nav}"
        );
        assert!(
            names.iter().any(|name| name == &format!("OEBPS/{href}")),
            "导航指向的 {href} 在 zip 里不存在——目录点了会 404"
        );
    }
    // 条目数与章节数一致：多章导出时目录不能只有一项。
    assert_eq!(
        nav.matches("<li>").count(),
        3,
        "导航条目数必须与章节数一致。nav=\n{nav}"
    );
}

/// 章节标题里的 XML 元字符必须被转义，否则 nav 非良构、整份目录解析失败。
#[test]
fn u127_nav_escapes_xml_metacharacters_in_titles() {
    let temp = tempfile::tempdir().unwrap();
    let bytes = export_three_chapter_epub(temp.path());
    let nav = read_entry(&bytes, "OEBPS/nav.xhtml");

    assert!(
        nav.contains("&amp;") && nav.contains("&lt;"),
        "标题里的 & 与 < 必须转义，否则 nav 变成非良构 XML、阅读器连目录都解析不出来。nav=\n{nav}"
    );
    // 裸的 ` & ` 一定是漏转义（合法实体形如 `&amp;`，不会以空格结尾）。
    assert!(
        !nav.contains(" & "),
        "nav 里出现未转义的裸 & 。nav=\n{nav}"
    );
}

/// nav **不入 spine**：目录页不该出现在正文阅读流里。
#[test]
fn u127_nav_is_not_part_of_the_reading_spine() {
    let temp = tempfile::tempdir().unwrap();
    let bytes = export_three_chapter_epub(temp.path());
    let opf = read_entry(&bytes, "OEBPS/content.opf");

    let spine = opf
        .split_once("<spine>")
        .and_then(|(_, rest)| rest.split_once("</spine>"))
        .map(|(spine, _)| spine.to_owned())
        .expect("content.opf 必须有 spine");
    assert!(
        !spine.contains(r#"idref="nav""#),
        "nav 不应进入 spine，否则读者翻正文时会先撞上一页目录。spine=\n{spine}"
    );
    // 章节仍必须全部在 spine 里——别为了排除 nav 把章节一起漏掉。
    for index in 0..3 {
        assert!(
            spine.contains(&format!(r#"idref="chapter{index}""#)),
            "第 {index} 章不在 spine 里。spine=\n{spine}"
        );
    }
}

/// 回归护栏：审查已实测合规的部分不得被本次改动弄坏。
///
/// `mimetype` 必须是**首个**条目且为 stored（无压缩、无 extra field），
/// 这是 OCF 的硬性要求；加 nav 时若不慎插到它前面，整个包立刻不合规。
#[test]
fn u127_ocf_mimetype_entry_stays_first_and_stored() {
    let temp = tempfile::tempdir().unwrap();
    let bytes = export_three_chapter_epub(temp.path());

    assert!(bytes.starts_with(b"PK\x03\x04"), "EPUB 必须是 zip");
    let names = entry_names(&bytes);
    assert_eq!(
        names.first().map(String::as_str),
        Some("mimetype"),
        "mimetype 必须是 zip 的第一个条目。实际顺序={names:?}"
    );

    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
    let mimetype = archive.by_name("mimetype").unwrap();
    assert_eq!(
        mimetype.compression(),
        zip::CompressionMethod::Stored,
        "mimetype 必须 stored 存储"
    );
    drop(mimetype);

    assert_eq!(read_entry(&bytes, "mimetype"), "application/epub+zip");
    // container.xml 指向的 rootfile 必须真实存在。
    let container = read_entry(&bytes, "META-INF/container.xml");
    assert!(container.contains("OEBPS/content.opf"));
    assert!(names.iter().any(|name| name == "OEBPS/content.opf"));
}
