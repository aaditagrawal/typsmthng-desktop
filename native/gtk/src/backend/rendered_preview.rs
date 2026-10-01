//! Cached SVG pages and presentation-note crops, independent of GTK.
use std::path::PathBuf;

use super::{BackendError, CompileOutput, Result, SvgPage};

pub struct PreparedPreview {
    pub cache: tempfile::TempDir,
    pub paths: Vec<PathBuf>,
    pub rendered_notes: Vec<Option<PathBuf>>,
}

pub fn prepare_preview(
    output: CompileOutput<Vec<SvgPage>>,
    notes_layout: &str,
) -> Result<CompileOutput<PreparedPreview>> {
    let artifact = output
        .artifact
        .map(|pages| -> Result<PreparedPreview> {
            let cache = tempfile::tempdir().map_err(|error| BackendError::Io {
                path: std::env::temp_dir(),
                source: error,
            })?;
            let split_notes = match notes_layout {
                "right-half" => true,
                "whole" => false,
                _ => {
                    !pages.is_empty()
                        && pages.iter().all(|page| {
                            page.width_points.zip(page.height_points).is_some_and(
                                |(width, height)| height > 0.0 && width / height >= 2.6,
                            )
                        })
                }
            };
            let mut paths = Vec::with_capacity(pages.len());
            let mut rendered_notes = Vec::with_capacity(pages.len());
            for page in pages {
                let path = cache.path().join(format!("page-{:04}.svg", page.page));
                let valid_dimensions =
                    page.width_points
                        .zip(page.height_points)
                        .is_some_and(|(width, height)| {
                            width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0
                        });
                let (slide_svg, note_svg) = if split_notes && valid_dimensions {
                    let width = page.width_points.unwrap_or_default();
                    let height = page.height_points.unwrap_or_default();
                    (
                        crop_svg(&page.svg, 0.0, width / 2.0, height),
                        Some(crop_svg(&page.svg, width / 2.0, width / 2.0, height)),
                    )
                } else {
                    (page.svg, None)
                };
                std::fs::write(&path, slide_svg).map_err(|error| BackendError::Io {
                    path: path.clone(),
                    source: error,
                })?;
                paths.push(path);
                let note_path = note_svg.and_then(|svg| {
                    let path = cache.path().join(format!("notes-{:04}.svg", page.page));
                    std::fs::write(&path, svg).ok().map(|()| path)
                });
                rendered_notes.push(note_path);
            }
            Ok(PreparedPreview {
                cache,
                paths,
                rendered_notes,
            })
        })
        .transpose()?;
    Ok(CompileOutput {
        artifact,
        diagnostics: output.diagnostics,
        stdout: output.stdout,
        stderr: output.stderr,
        elapsed: output.elapsed,
    })
}

fn crop_svg(svg: &str, x: f64, width: f64, height: f64) -> String {
    let view_box = regex::Regex::new(r#"\bviewBox="[^"]*""#).unwrap();
    let root_width = regex::Regex::new(r#"\bwidth="[0-9.]+(?:pt)?""#).unwrap();
    let svg = view_box
        .replacen(
            svg,
            1,
            format!("viewBox=\"{x:.3} 0 {width:.3} {height:.3}\""),
        )
        .into_owned();
    root_width
        .replacen(&svg, 1, format!("width=\"{width:.3}pt\""))
        .into_owned()
}

#[cfg(test)]
mod tests {
    use std::fs;

    fn preview_output(dimensions: &[(f64, f64)]) -> super::CompileOutput<Vec<super::SvgPage>> {
        super::CompileOutput {
            artifact: Some(dimensions.iter().enumerate().map(|(index, &(width, height))| {
                super::SvgPage {
                    page: index + 1,
                    svg: format!(r#"<svg width="{width}pt" height="{height}pt" viewBox="0 0 {width} {height}"><text>slide</text></svg>"#),
                    width_points: Some(width),
                    height_points: Some(height),
                }
            }).collect()),
            diagnostics: Vec::new(),
            stdout: "compiler output".into(),
            stderr: String::new(),
            elapsed: std::time::Duration::from_millis(12),
        }
    }

    #[test]
    fn prepared_preview_splits_ultrawide_notes_and_owns_cache_lifetime() {
        let result = super::prepare_preview(preview_output(&[(1200.0, 400.0)]), "auto").unwrap();
        assert_eq!(result.stdout, "compiler output");
        assert_eq!(result.elapsed.as_millis(), 12);
        let preview = result.artifact.unwrap();
        let slide = fs::read_to_string(&preview.paths[0]).unwrap();
        assert!(slide.contains(r#"viewBox="0.000 0 600.000 400.000""#));
        let notes = fs::read_to_string(preview.rendered_notes[0].as_ref().unwrap()).unwrap();
        assert!(notes.contains(r#"viewBox="600.000 0 600.000 400.000""#));
        let cache_path = preview.cache.path().to_path_buf();
        assert!(cache_path.is_dir());
        drop(preview);
        assert!(!cache_path.exists());
    }

    #[test]
    fn prepared_preview_respects_mixed_pages_and_layout_overrides() {
        for (layout, dimensions, split) in [
            ("auto", vec![(1200.0, 400.0), (600.0, 800.0)], false),
            ("whole", vec![(1200.0, 400.0)], false),
            ("right-half", vec![(1200.0, 800.0)], true),
        ] {
            let preview = super::prepare_preview(preview_output(&dimensions), layout)
                .unwrap()
                .artifact
                .unwrap();
            assert_eq!(preview.paths.len(), dimensions.len());
            assert!(preview
                .rendered_notes
                .iter()
                .all(|path| path.is_some() == split));
        }
        let mut failed = preview_output(&[]);
        failed.artifact = None;
        failed.stderr = "compile failure".into();
        let result = super::prepare_preview(failed, "auto").unwrap();
        assert!(result.artifact.is_none());
        assert_eq!(result.stderr, "compile failure");
    }

    #[test]
    fn forced_notes_split_preserves_pages_without_valid_dimensions() {
        for dimensions in [
            (None, Some(400.0)),
            (Some(1200.0), None),
            (Some(0.0), Some(400.0)),
            (Some(f64::NAN), Some(400.0)),
        ] {
            let mut output = preview_output(&[(1200.0, 400.0)]);
            let page = &mut output.artifact.as_mut().unwrap()[0];
            page.width_points = dimensions.0;
            page.height_points = dimensions.1;
            let original = page.svg.clone();
            let preview = super::prepare_preview(output, "right-half")
                .unwrap()
                .artifact
                .unwrap();
            assert_eq!(fs::read_to_string(&preview.paths[0]).unwrap(), original);
            assert!(preview.rendered_notes[0].is_none());
        }
    }
}
