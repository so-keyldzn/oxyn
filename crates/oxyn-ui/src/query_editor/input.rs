//! Platform text input and composition for the SQL editor.
use super::*;
use gpui::{EntityInputHandler, UTF16Selection, point};

fn position_from_utf16(buffer: &TextBuffer, offset: usize) -> TextPosition {
    let mut remaining = offset;
    for (line, text) in buffer.lines.iter().enumerate() {
        for (column, ch) in text.chars().enumerate() {
            if remaining == 0 {
                return TextPosition::new(line, column);
            }
            remaining = remaining.saturating_sub(ch.len_utf16());
        }
        if remaining == 0 {
            return TextPosition::new(line, text.chars().count());
        }
        remaining = remaining.saturating_sub(1);
    }
    buffer.end()
}
fn utf16_at(buffer: &TextBuffer, position: TextPosition) -> usize {
    let position = buffer.clamp(position);
    buffer
        .lines
        .iter()
        .take(position.line)
        .map(|line| line.encode_utf16().count() + 1)
        .sum::<usize>()
        + buffer
            .line(position.line)
            .unwrap_or_default()
            .chars()
            .take(position.column)
            .map(char::len_utf16)
            .sum::<usize>()
}

impl QueryEditor {
    fn native_selection(&self) -> Range<usize> {
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        utf16_at(&self.buffer, start)..utf16_at(&self.buffer, end)
    }
}

impl EntityInputHandler for QueryEditor {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<String> {
        let start = position_from_utf16(&self.buffer, range.start.min(range.end));
        let end = position_from_utf16(&self.buffer, range.end.max(range.start));
        *actual = Some(utf16_at(&self.buffer, start)..utf16_at(&self.buffer, end));
        Some(self.buffer.slice(start, end))
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.native_selection(),
            reversed: self.anchor.is_some_and(|anchor| self.cursor < anchor),
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<'_, Self>) -> Option<Range<usize>> {
        self.marked.clone()
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<'_, Self>) {
        self.marked = None;
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.read_only {
            return;
        }
        let range = range
            .or(self.marked.take())
            .unwrap_or_else(|| self.native_selection());
        self.anchor = Some(position_from_utf16(&self.buffer, range.start));
        self.cursor = position_from_utf16(&self.buffer, range.end);
        self.insert(text, cx);
        self.marked = None;
        self.scroll
            .scroll_to_item(self.cursor.line, gpui::ScrollStrategy::Top);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.read_only {
            return;
        }
        let range = range
            .or(self.marked.take())
            .unwrap_or_else(|| self.native_selection());
        let start = range.start;
        self.replace_text_in_range(Some(range), text, window, cx);
        let length = text.encode_utf16().count();
        self.marked = (length > 0).then_some(start..start + length);
        if let Some(selection) = selection {
            self.anchor = Some(position_from_utf16(
                &self.buffer,
                start + selection.start.min(length),
            ));
            self.cursor = position_from_utf16(&self.buffer, start + selection.end.min(length));
        }
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<Bounds<Pixels>> {
        let start = position_from_utf16(&self.buffer, range.start);
        let end = position_from_utf16(&self.buffer, range.end);
        let (line, bounds) = self.line_layouts.get(&start.line)?;
        let (gutter, padding) = self.mouse_metrics();
        let byte_at = |column| {
            line.text
                .char_indices()
                .nth(column)
                .map_or(line.text.len(), |(byte, _)| byte)
        };
        let right = if end.line == start.line {
            line.x_for_index(byte_at(end.column))
        } else {
            bounds.size.width
        };
        Some(Bounds::from_corners(
            point(
                bounds.left() + gutter + padding + line.x_for_index(byte_at(start.column)),
                bounds.top(),
            ),
            point(bounds.left() + gutter + padding + right, bounds.bottom()),
        ))
    }
    fn character_index_for_point(
        &mut self,
        position: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<usize> {
        let (&line, _) = self
            .line_layouts
            .iter()
            .find(|(_, (_, bounds))| position.y >= bounds.top() && position.y <= bounds.bottom())?;
        Some(utf16_at(
            &self.buffer,
            TextPosition::new(line, self.mouse_column(line, position.x)),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_offsets_cover_multiline_unicode_and_out_of_range_input() {
        let buffer = TextBuffer::from_text("SELECT '😀';\né");
        let position = TextPosition::new(1, 1);
        let offset = utf16_at(&buffer, position);
        assert_eq!(position_from_utf16(&buffer, offset), position);
        assert_eq!(position_from_utf16(&buffer, usize::MAX), buffer.end());
        for line in 0..buffer.line_count() {
            for column in 0..=buffer.line_len(line) {
                let position = TextPosition::new(line, column);
                assert_eq!(
                    position_from_utf16(&buffer, utf16_at(&buffer, position)),
                    position
                );
            }
        }
    }
}
