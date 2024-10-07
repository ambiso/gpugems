use std::{io::Cursor, time::Duration};

use reqwest::Url;
use soup::prelude::*;
use tokio::{
    fs::{create_dir_all, File},
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter},
    process::Command,
};

async fn scape_soup<S: AsyncWrite + Unpin>(soup: &Soup, f: &mut S) -> anyhow::Result<()> {
    for c in soup.tag("script") {
        f.write_all(c.display().as_bytes()).await?;
    }
    for c in soup.tag("link") {
        f.write_all(c.display().as_bytes()).await?;
    }
    let header = soup.tag("div").attr("id", "book_header").find().unwrap();
    for c in header.parent().unwrap().children().filter(|x| {
        x.attrs()
            .get("id")
            .map(|id| !["book_switch", "book_header"].contains(&id.as_str()))
            .unwrap_or(true)
    }) {
        if c.tag("h4")
            .find()
            .map(|x| x.text().trim() == "Copyright")
            .unwrap_or(false)
        {
            break;
        }
        f.write_all(c.display().as_bytes()).await?;
    }
    f.write_all(
        concat!(
            "<script>",
            concat!(include_str!("../inject.js"), "</script>")
        )
        .as_bytes(),
    )
    .await?;
    Ok(())
}

async fn get_soup(url: &str) -> anyhow::Result<Soup> {
    let url = Url::parse(url)?;
    let path = format!("./{CACHE_PATH}/{}", url.path());
    create_dir_all(&path).await?; // WTF

    let fpath = format!("{path}/index.html");
    match File::open(&fpath).await {
        Ok(f) => {
            let mut buf = Vec::new();
            BufReader::new(f).read_to_end(&mut buf).await?;
            Ok(Soup::from_reader(Cursor::new(buf))?)
        }
        _ => {
            println!("Fetching...");
            tokio::time::sleep(Duration::from_secs(1)).await;
            let r = reqwest::get(url).await?;
            let bytes = r.bytes().await?;
            let mut f = BufWriter::new(File::create_new(&fpath).await?);
            f.write_all(&bytes).await?;
            f.flush().await?;
            Ok(Soup::from_reader(Cursor::new(bytes))?)
        }
    }
}

const CACHE_PATH: &str = "./cache";

async fn build_pdf(s: &str) -> anyhow::Result<()> {
    create_dir_all(CACHE_PATH).await?;

    let path = format!("/gpugems/{s}/");
    let soup = get_soup(&format!("https://developer.nvidia.com{path}")).await?;
    let links = soup.tag("a").find_all();
    let mut chapter_links = vec![];

    for link in links {
        for (attr, value) in link.attrs() {
            if attr == "href" && value.starts_with(&path) {
                chapter_links.push((value, link.text()));
            }
        }
    }

    let pdf_dir = format!("pdfs/{s}");
    create_dir_all(&pdf_dir).await?;
    let html_dir = format!("htmls/{s}");
    create_dir_all(&html_dir).await?;

    let mut chapter_meta = vec![];
    let mut pdf_names = vec![];
    for (i, (url, chapter_name)) in chapter_links.iter().enumerate() {
        let cleaned_fn = format!(
            "{i:02}_{}",
            chapter_name.replace(|x: char| !x.is_ascii_alphanumeric(), "_")
        );
        let html_path = format!("{html_dir}/{cleaned_fn}.html");
        let mut f = BufWriter::new(File::create(&html_path).await?);
        let mut level = 1;
        if url
            .strip_prefix(&path)
            .map(|x| x.contains("/"))
            .unwrap_or(false)
        {
            level = 2;
            print!("    ");
        }
        println!("{chapter_name}");
        let soup = get_soup(&format!("https://developer.nvidia.com/{url}")).await?;

        f.write_all(
            concat!("<style>", concat!(include_str!("../style.css"), "</style>")).as_bytes(),
        )
        .await?;
        scape_soup(&soup, &mut f).await?;
        f.flush().await?;
        let fname = format!("{pdf_dir}/{cleaned_fn}.pdf",);
        pdf_names.push(fname);
        let fname = pdf_names.last().unwrap();
        Command::new("chromium")
            .args(&[
                "--headless",
                // "--disable-gpu",
                "--run-all-compositor-stages-before-draw",
                "--no-pdf-header-footer",
                "--print-to-pdf-no-header",
                &format!("--print-to-pdf={fname}"),
                &html_path,
            ])
            .status()
            .await?;
        Command::new("pdftk")
            .args(&[&fname, "dump_data", "output", "meta.txt"])
            .status()
            .await?;

        let f = BufReader::new(File::open("meta.txt").await?);
        let mut lines = f.lines();
        while let Ok(Some(l)) = lines.next_line().await {
            let mut found = false;
            let mut pages = 0;
            for x in l.split(":") {
                if found {
                    pages = x.trim().parse()?;
                    break;
                }
                if x != "NumberOfPages" {
                    break;
                }
                found = true;
            }
            if found {
                chapter_meta.push((level, pages, chapter_name));
                break;
            }
        }
    }

    // merge PDFs
    let mut args = pdf_names;
    args.extend(
        ["cat", "output", "merged.pdf"]
            .iter()
            .map(|x| x.to_string()),
    );
    Command::new("pdftk").args(args).status().await?;

    // extract metadata from merged PDF
    Command::new("pdftk")
        .args(&["merged.pdf", "dump_data", "output", "meta.txt"])
        .status()
        .await?;

    let mut pages_sum = 1;
    let mut f = BufWriter::new(File::options().append(true).open("meta.txt").await?);
    for (level, pages, chapter_name) in chapter_meta.iter() {
        f.write_all(
            format!("BookmarkBegin\nBookmarkTitle: {chapter_name}\nBookmarkLevel: {level}\nBookmarkPageNumber: {pages_sum}\n")
                .as_bytes(),
        )
        .await?;
        pages_sum += pages;
    }
    f.flush().await?;
    drop(f);

    Command::new("pdftk")
        .args(&[
            "merged.pdf",
            "update_info",
            "meta.txt",
            "output",
            &format!("{s}.pdf"),
        ])
        .status()
        .await?;
    println!("Done");
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    build_pdf("gpugems").await?;
    build_pdf("gpugems2").await?;
    build_pdf("gpugems3").await?;
    Ok(())
}
