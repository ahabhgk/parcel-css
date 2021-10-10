use cssparser::*;

/// A type that encapsulates a media query list.
#[derive(Clone, Debug)]
pub struct MediaList {
    /// The list of media queries.
    pub media_queries: Vec<MediaQuery>,
}

impl MediaList {
  /// Parse a media query list from CSS.
  ///
  /// Always returns a media query list. If any invalid media query is
  /// found, the media query list is only filled with the equivalent of
  /// "not all", see:
  ///
  /// <https://drafts.csswg.org/mediaqueries/#error-handling>
  pub fn parse(input: &mut Parser) -> Self {
      // if input.is_exhausted() {
      //     return Self::empty();
      // }

      let mut media_queries = vec![];
      loop {
          let start_position = input.position();
          match input.parse_until_before(Delimiter::Comma, |i| MediaQuery::parse(i)) {
              Ok(mq) => {
                  media_queries.push(mq);
              },
              Err(err) => {
                println!("{:?}", err);
                  // media_queries.push(MediaQuery::never_matching());
                  // let location = err.location;
                  // let error = ContextualParseError::InvalidMediaRule(
                  //     input.slice_from(start_position),
                  //     err,
                  // );
                  // context.log_css_error(location, error);
              },
          }

          match input.next() {
              Ok(&Token::Comma) => {},
              Ok(_) => unreachable!(),
              Err(_) => break,
          }
      }

      MediaList { media_queries }
  }
}

/// <https://drafts.csswg.org/mediaqueries/#mq-prefix>
#[derive(Clone, Copy, Debug)]
pub enum Qualifier {
    /// Hide a media query from legacy UAs:
    /// <https://drafts.csswg.org/mediaqueries/#mq-only>
    Only,
    /// Negate a media query:
    /// <https://drafts.csswg.org/mediaqueries/#mq-not>
    Not,
}

impl Qualifier {
  pub fn parse<'i, 't>(
    input: &mut Parser<'i, 't>,
  ) -> Result<Self, ParseError<'i, ()>> {
    let location = input.current_source_location();
    let ident = input.expect_ident()?;
    match_ignore_ascii_case! { &*ident,
      "only" => Ok(Qualifier::Only),
      "not" => Ok(Qualifier::Not),
      _ => Err(location.new_unexpected_token_error(
        cssparser::Token::Ident(ident.clone())
      ))
    }
  }
}

/// <http://dev.w3.org/csswg/mediaqueries-3/#media0>
#[derive(Clone, Debug)]
pub enum MediaType {
    /// A media type that matches every device.
    All,
    Print,
    Screen,
    /// A specific media type.
    Custom(String),
}

impl MediaType {
  fn parse(name: &str) -> Result<Self, ()> {
    match_ignore_ascii_case! { &*name,
      "all" => Ok(MediaType::All),
      "print" => Ok(MediaType::Print),
      "screen" => Ok(MediaType::Screen),
      _ => Ok(MediaType::Custom(name.into()))
    }
  }
}

/// A [media query][mq].
///
/// [mq]: https://drafts.csswg.org/mediaqueries/
#[derive(Clone, Debug)]
pub struct MediaQuery {
    /// The qualifier for this query.
    pub qualifier: Option<Qualifier>,
    /// The media type for this query, that can be known, unknown, or "all".
    pub media_type: MediaType,
    /// The condition that this media query contains. This cannot have `or`
    /// in the first level.
    pub condition: Option<MediaCondition>,
}

impl MediaQuery {
  /// Parse a media query given css input.
  ///
  /// Returns an error if any of the expressions is unknown.
  pub fn parse<'i, 't>(
    input: &mut Parser<'i, 't>,
  ) -> Result<Self, ParseError<'i, ()>> {
    let (qualifier, explicit_media_type) = input
        .try_parse(|input| -> Result<_, ()> {
            let qualifier = input.try_parse(Qualifier::parse).ok();
            let ident = input.expect_ident().map_err(|_| ())?;
            let media_type = MediaType::parse(&ident)?;
            Ok((qualifier, Some(media_type)))
        })
        .unwrap_or_default();

    let condition = if explicit_media_type.is_none() {
        Some(MediaCondition::parse(input, true)?)
    } else if input.try_parse(|i| i.expect_ident_matching("and")).is_ok() {
        Some(MediaCondition::parse(input, false)?)
    } else {
        None
    };

    let media_type = explicit_media_type.unwrap_or(MediaType::All);
    Ok(Self {
        qualifier,
        media_type,
        condition,
    })
  }
}

/// A binary `and` or `or` operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(missing_docs)]
pub enum Operator {
    And,
    Or,
}

impl Operator {
  pub fn parse<'i, 't>(
    input: &mut Parser<'i, 't>,
  ) -> Result<Self, ParseError<'i, ()>> {
    let location = input.current_source_location();
    let ident = input.expect_ident()?;
    match_ignore_ascii_case! { &*ident,
      "and" => Ok(Operator::And),
      "or" => Ok(Operator::Or),
      _ => Err(location.new_unexpected_token_error(
        cssparser::Token::Ident(ident.clone())
      ))
    }
  }
}

/// Represents a media condition.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaCondition {
    /// A simple media feature expression, implicitly parenthesized.
    Feature(MediaFeatureExpression),
    /// A negation of a condition.
    Not(Box<MediaCondition>),
    /// A set of joint operations.
    Operation(Box<[MediaCondition]>, Operator),
    /// A condition wrapped in parenthesis.
    InParens(Box<MediaCondition>),
}

impl MediaCondition {
  /// Parse a single media condition.
  pub fn parse<'i, 't>(
      input: &mut Parser<'i, 't>,
      allow_or: bool
  ) -> Result<Self, ParseError<'i, ()>> {
    let location = input.current_source_location();

    // FIXME(emilio): This can be cleaner with nll.
    let is_negation = match *input.next()? {
        Token::ParenthesisBlock => false,
        Token::Ident(ref ident) if ident.eq_ignore_ascii_case("not") => true,
        ref t => return Err(location.new_unexpected_token_error(t.clone())),
    };

    if is_negation {
        let inner_condition = Self::parse_in_parens(input)?;
        return Ok(MediaCondition::Not(Box::new(inner_condition)));
    }

    // ParenthesisBlock.
    let first_condition = Self::parse_paren_block(input)?;
    let operator = match input.try_parse(Operator::parse) {
        Ok(op) => op,
        Err(..) => return Ok(first_condition),
    };

    if allow_or && operator == Operator::Or {
        // return Err(location.new_custom_error(StyleParseErrorKind::UnspecifiedError));
    }

    let mut conditions = vec![];
    conditions.push(first_condition);
    conditions.push(Self::parse_in_parens(input)?);

    let delim = match operator {
        Operator::And => "and",
        Operator::Or => "or",
    };

    loop {
        if input.try_parse(|i| i.expect_ident_matching(delim)).is_err() {
            return Ok(MediaCondition::Operation(
                conditions.into_boxed_slice(),
                operator,
            ));
        }

        conditions.push(Self::parse_in_parens(input)?);
    }
  }

  /// Parse a media condition in parentheses.
  pub fn parse_in_parens<'i, 't>(
      input: &mut Parser<'i, 't>,
  ) -> Result<Self, ParseError<'i, ()>> {
      input.expect_parenthesis_block()?;
      Self::parse_paren_block(input)
  }

  fn parse_paren_block<'i, 't>(
      input: &mut Parser<'i, 't>,
  ) -> Result<Self, ParseError<'i, ()>> {
      input.parse_nested_block(|input| {
          // Base case.
          if let Ok(inner) = input.try_parse(|i| Self::parse(i, true)) {
              return Ok(MediaCondition::InParens(Box::new(inner)));
          }
          let expr = MediaFeatureExpression::parse_in_parenthesis_block(input)?;
          Ok(MediaCondition::Feature(expr))
      })
  }
}

/// The operator that was specified in this media feature.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaQueryOperator {
    /// =
    Equal,
    /// >
    GreaterThan,
    /// >=
    GreaterThanEqual,
    /// <
    LessThan,
    /// <=
    LessThanEqual,
}

/// A feature expression contains a reference to the media feature, the value
/// the media query contained, and the range to evaluate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaFeatureExpression {
  name: String,
  operator: Option<MediaQueryOperator>,
  value: Option<String>
    // feature_index: usize,
    // value: Option<MediaExpressionValue>,
    // range_or_operator: Option<RangeOrOperator>,
}

impl MediaFeatureExpression {
  /// Parse a media feature expression where we've already consumed the
  /// parenthesis.
  pub fn parse_in_parenthesis_block<'i, 't>(
      input: &mut Parser<'i, 't>,
  ) -> Result<Self, ParseError<'i, ()>> {
      // let mut requirements = ParsingRequirements::empty();
      let location = input.current_source_location();
      let ident = input.expect_ident()?;

      // if context.in_ua_or_chrome_sheet() {
      //     requirements.insert(ParsingRequirements::CHROME_AND_UA_ONLY);
      // }

      let feature_name = String::from(&**ident);

      // if starts_with_ignore_ascii_case(feature_name, "-webkit-") {
      //     feature_name = &feature_name[8..];
      //     requirements.insert(ParsingRequirements::WEBKIT_PREFIX);
      // }

      // let range = if starts_with_ignore_ascii_case(feature_name, "min-") {
      //     feature_name = &feature_name[4..];
      //     Some(Range::Min)
      // } else if starts_with_ignore_ascii_case(feature_name, "max-") {
      //     feature_name = &feature_name[4..];
      //     Some(Range::Max)
      // } else {
      //     None
      // };

      // let atom = Atom::from(string_as_ascii_lowercase(feature_name));

      // let (feature_index, feature) = match MEDIA_FEATURES
      //     .iter()
      //     .enumerate()
      //     .find(|(_, f)| f.name == atom)
      // {
      //     Some((i, f)) => (i, f),
      //     None => {
      //         return Err(location.new_custom_error(
      //             StyleParseErrorKind::MediaQueryExpectedFeatureName(ident.clone()),
      //         ))
      //     },
      // };

      // if disabled_by_pref(&feature.name, context) ||
      //     !requirements.contains(feature.requirements) ||
      //     (range.is_some() && !feature.allows_ranges())
      // {
      //     return Err(location.new_custom_error(
      //         StyleParseErrorKind::MediaQueryExpectedFeatureName(ident.clone()),
      //     ));
      // }

      let operator = input.try_parse(consume_operation_or_colon);
      let operator = match operator {
          Err(..) => {
              // If there's no colon, this is a media query of the
              // form '(<feature>)', that is, there's no value
              // specified.
              //
              // Gecko doesn't allow ranged expressions without a
              // value, so just reject them here too.
              // if range.is_some() {
              //     return Err(
              //         input.new_custom_error(StyleParseErrorKind::RangedExpressionWithNoValue)
              //     );
              // }

              // return Ok(Self::new(feature_index, None, None));
              return Ok(MediaFeatureExpression {
                name: feature_name,
                operator: None,
                value: None
              })
          },
          Ok(operator) => operator,
      };

      // let range_or_operator = match range {
      //     Some(range) => {
      //         if operator.is_some() {
      //             return Err(
      //                 input.new_custom_error(StyleParseErrorKind::MediaQueryUnexpectedOperator)
      //             );
      //         }
      //         Some(RangeOrOperator::Range(range))
      //     },
      //     None => match operator {
      //         Some(operator) => {
      //             if !feature.allows_ranges() {
      //                 return Err(input
      //                     .new_custom_error(StyleParseErrorKind::MediaQueryUnexpectedOperator));
      //             }
      //             Some(RangeOrOperator::Operator(operator))
      //         },
      //         None => None,
      //     },
      // };

      // let value = MediaExpressionValue::parse(feature, context, input).map_err(|err| {
      //     err.location
      //         .new_custom_error(StyleParseErrorKind::MediaQueryExpectedFeatureValue)
      // })?;
      let value = exhaust(input);

      Ok(MediaFeatureExpression {
        name: feature_name.into(),
        operator,
        value: Some(value.into())
      })
  }
}

fn exhaust<'i>(input: &mut cssparser::Parser<'i, '_>) -> &'i str {
  let start = input.position();
  while input.next().is_ok() {}
  input.slice_from(start)
}

/// Consumes an operation or a colon, or returns an error.
fn consume_operation_or_colon(input: &mut Parser) -> Result<Option<MediaQueryOperator>, ()> {
  let first_delim = {
      let next_token = match input.next() {
          Ok(t) => t,
          Err(..) => return Err(()),
      };

      match *next_token {
          Token::Colon => return Ok(None),
          Token::Delim(oper) => oper,
          _ => return Err(()),
      }
  };
  Ok(Some(match first_delim {
      '=' => MediaQueryOperator::Equal,
      '>' => {
          if input.try_parse(|i| i.expect_delim('=')).is_ok() {
            MediaQueryOperator::GreaterThanEqual
          } else {
            MediaQueryOperator::GreaterThan
          }
      },
      '<' => {
          if input.try_parse(|i| i.expect_delim('=')).is_ok() {
            MediaQueryOperator::LessThanEqual
          } else {
            MediaQueryOperator::LessThan
          }
      },
      _ => return Err(()),
  }))
}