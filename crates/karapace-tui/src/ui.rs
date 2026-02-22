use crate::app::{App, InputMode, View};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
};

pub fn draw(f: &mut Frame<'_>, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_header(f, chunks[0]);

    match app.view {
        View::List => draw_list(f, app, chunks[1]),
        View::Detail => draw_detail(f, app, chunks[1]),
        View::Help => draw_help(f, chunks[1]),
    }

    draw_status_bar(f, app, chunks[2]);
}

fn draw_header(f: &mut Frame<'_>, area: Rect) {
    let title = Paragraph::new(format!(
        " Karapace Environment Manager  v{}",
        env!("CARGO_PKG_VERSION")
    ))
    .style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(title, area);
}

fn draw_list(f: &mut Frame<'_>, app: &App, area: Rect) {
    if app.environments.is_empty() {
        let msg = Paragraph::new("  No environments found. Press 'q' to quit.").block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Environments "),
        );
        f.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from("SHORT_ID").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("NAME").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("STATE").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("ENV_ID").style(Style::default().add_modifier(Modifier::BOLD)),
    ])
    .height(1);

    let rows: Vec<Row<'_>> = app
        .filtered
        .iter()
        .enumerate()
        .map(|(vi, &ei)| {
            let env = &app.environments[ei];
            let style = if vi == app.selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let state_style = state_color(&env.state.to_string());
            Row::new(vec![
                Cell::from(env.short_id.to_string()),
                Cell::from(env.name.as_deref().unwrap_or("").to_owned()),
                Cell::from(env.state.to_string()).style(state_style),
                Cell::from(env.env_id.to_string()),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(14),
            Constraint::Length(16),
            Constraint::Length(10),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(format!(
        " Environments ({}/{}) ",
        app.visible_count(),
        app.environments.len()
    )));

    f.render_widget(table, area);
}

fn draw_detail(f: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(env) = app.selected_env() else {
        let msg = Paragraph::new("  No environment selected.")
            .block(Block::default().borders(Borders::ALL).title(" Detail "));
        f.render_widget(msg, area);
        return;
    };

    let text = vec![
        Line::from(vec![
            Span::styled(
                "env_id:      ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(env.env_id.to_string()),
        ]),
        Line::from(vec![
            Span::styled(
                "short_id:    ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(env.short_id.to_string()),
        ]),
        Line::from(vec![
            Span::styled(
                "name:        ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(env.name.as_deref().unwrap_or("(none)")),
        ]),
        Line::from(vec![
            Span::styled(
                "state:       ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(env.state.to_string(), state_color(&env.state.to_string())),
        ]),
        Line::from(vec![
            Span::styled(
                "base_layer:  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(env.base_layer.to_string()),
        ]),
        Line::from(vec![
            Span::styled(
                "deps:        ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(env.dependency_layers.len().to_string()),
        ]),
        Line::from(vec![
            Span::styled(
                "ref_count:   ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(env.ref_count.to_string()),
        ]),
        Line::from(vec![
            Span::styled(
                "created_at:  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(&env.created_at),
        ]),
        Line::from(vec![
            Span::styled(
                "updated_at:  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(&env.updated_at),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  [Esc] back  [d] destroy  [f] freeze  [a] archive  [n] rename",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let detail = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(format!(
            " {} ",
            env.name.as_deref().unwrap_or(&env.short_id)
        )))
        .wrap(Wrap { trim: false });

    f.render_widget(detail, area);
}

fn draw_help(f: &mut Frame<'_>, area: Rect) {
    let text = vec![
        Line::from(Span::styled(
            "Keybindings",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  j / ↓       Move down"),
        Line::from("  k / ↑       Move up"),
        Line::from("  g / Home    Go to top"),
        Line::from("  G / End     Go to bottom"),
        Line::from("  Enter       View details"),
        Line::from("  d           Destroy (with confirm)"),
        Line::from("  f           Freeze environment"),
        Line::from("  a           Archive environment"),
        Line::from("  n           Rename environment"),
        Line::from("  /           Search / filter"),
        Line::from("  s           Cycle sort column"),
        Line::from("  S           Toggle sort direction"),
        Line::from("  r           Refresh list"),
        Line::from("  ?           Show this help"),
        Line::from("  q / Esc     Quit / Back"),
    ];

    let help = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(" Help "))
        .wrap(Wrap { trim: false });

    f.render_widget(help, area);
}

fn draw_status_bar(f: &mut Frame<'_>, app: &App, area: Rect) {
    let status = if app.show_confirm.is_some() || app.input_mode != InputMode::Normal {
        Paragraph::new(format!(" {} ", app.status_message)).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Paragraph::new(format!(
            " {} │ [j/k] nav  [Enter] detail  [d] destroy  [f] freeze  [/] search  [?] help  [q] quit",
            app.status_message
        ))
        .style(Style::default().fg(Color::DarkGray))
    };
    f.render_widget(status, area);
}

fn state_color(state: &str) -> Style {
    match state {
        "built" => Style::default().fg(Color::Green),
        "running" => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        "defined" => Style::default().fg(Color::Yellow),
        "frozen" => Style::default().fg(Color::Blue),
        "archived" => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    }
}
