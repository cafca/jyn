import 'package:flutter_test/flutter_test.dart';
import 'package:jyn/src/actions.dart';
import 'package:jyn/src/rust/state.dart';
import 'package:jyn/src/widgets/jyn_avatar.dart';

RiverHeart _heart(String name) =>
    RiverHeart(hearterProfileId: name.toLowerCase(), hearterDisplayName: name);

void main() {
  group('heartsSummary', () {
    test('names, never a bare count', () {
      expect(heartsSummary([]), '');
      expect(heartsSummary([_heart('Mira')]), 'Mira');
      expect(
        heartsSummary([_heart('Mira'), _heart('Soren')]),
        'Mira and Soren',
      );
      expect(
        heartsSummary([_heart('Mira'), _heart('Soren'), _heart('Bo')]),
        'Mira, Soren and others',
      );
    });
  });

  group('avatar derivation', () {
    test('gradient is deterministic per profile id', () {
      expect(gradientForProfile('abc123'), gradientForProfile('abc123'));
      expect(gradientForProfile('abc123'), isNot(gradientForProfile('def456')));
    });

    test('initials come from the first two words', () {
      expect(initialsForName('Ren Okafor'), 'RO');
      expect(initialsForName('Mira'), 'MI');
      expect(initialsForName('  June  Park  '), 'JP');
      expect(initialsForName('x'), 'X');
      expect(initialsForName(''), '?');
    });
  });
}
